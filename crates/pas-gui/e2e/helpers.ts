import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote, type Browser } from "webdriverio";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(__dirname, "../../..");
export const E2E_TARGET_DEBUG = path.join(REPO_ROOT, "target/e2e/debug");
export const PAS_GUI_BIN = path.join(E2E_TARGET_DEBUG, "pas-gui");
export const PASD_BIN = path.join(E2E_TARGET_DEBUG, "pasd");
export const PAS_CLI_BIN = path.join(E2E_TARGET_DEBUG, "pas");
export const FAKE_AGENT_BIN = path.join(E2E_TARGET_DEBUG, "fake-agent");
export const TAURI_DRIVER_BIN =
  process.env.TAURI_DRIVER_BIN || path.join(os.homedir(), ".cargo/bin/tauri-driver");
export const NATIVE_DRIVER_BIN =
  process.env.WEBKIT_WEBDRIVER_BIN || "/usr/bin/WebKitWebDriver";

function ensurePrerequisites() {
  const required = [
    ["pas-gui", PAS_GUI_BIN],
    ["pasd", PASD_BIN],
    ["pas", PAS_CLI_BIN],
    ["fake-agent", FAKE_AGENT_BIN],
    ["tauri-driver", TAURI_DRIVER_BIN],
    ["WebKitWebDriver", NATIVE_DRIVER_BIN],
  ] as const;
  for (const [label, binPath] of required) {
    if (!fs.existsSync(binPath)) {
      throw new Error(
        `No se encontró ${label} en ${binPath}. Corré \`npm run e2e\` para compilar target/e2e.`,
      );
    }
  }
}

function getFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      if (!addr || typeof addr === "string") {
        server.close();
        reject(new Error("No se pudo obtener un puerto TCP libre."));
        return;
      }
      const { port } = addr;
      server.close((err) => (err ? reject(err) : resolve(port)));
    });
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function sendSignal(pid: number, signal: NodeJS.Signals) {
  // Como lanzamos con detached: true, -pid apunta exactamente al process group del proceso hijo.
  try {
    process.kill(-pid, signal);
  } catch {
    // El grupo puede no existir si ya salió.
  }
  try {
    process.kill(pid, signal);
  } catch {
    // El proceso ya salió.
  }
}

async function terminateProcessByPid(pid: number, timeoutMs = 4_000): Promise<void> {
  if (!isPidAlive(pid)) return;
  sendSignal(pid, "SIGTERM");
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!isPidAlive(pid)) return;
    await sleep(50);
  }
  if (isPidAlive(pid)) {
    sendSignal(pid, "SIGKILL");
    const killDeadline = Date.now() + 1_000;
    while (Date.now() < killDeadline && isPidAlive(pid)) {
      await sleep(50);
    }
  }
}

async function waitForUnixSocket(socketPath: string, timeoutMs = 5_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (fs.existsSync(socketPath)) {
      const ok = await new Promise<boolean>((resolve) => {
        const client = net.createConnection(socketPath);
        client.once("connect", () => {
          client.destroy();
          resolve(true);
        });
        client.once("error", () => {
          client.destroy();
          resolve(false);
        });
      });
      if (ok) return;
    }
    await sleep(50);
  }
  throw new Error(`pasd no escuchó en ${socketPath} dentro de ${timeoutMs} ms.`);
}

async function waitForTcpPort(port: number, timeoutMs = 5_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const ok = await new Promise<boolean>((resolve) => {
      const client = net.createConnection({ host: "127.0.0.1", port });
      client.once("connect", () => {
        client.destroy();
        resolve(true);
      });
      client.once("error", () => {
        client.destroy();
        resolve(false);
      });
    });
    if (ok) return;
    await sleep(50);
  }
  throw new Error(`tauri-driver no escuchó en 127.0.0.1:${port} dentro de ${timeoutMs} ms.`);
}

export interface E2EContextOptions {
  pasdBinOverride?: string;
  extraPathDir?: string;
}

export class E2EContext {
  readonly tmpDir: string;
  readonly socketPath: string;
  readonly xdgDataHome: string;
  readonly pasDataDir: string;
  readonly layoutFile: string;
  private readonly pasdBinOverride?: string;
  private extraPathDirs: string[] = [];

  private pasdProc: ChildProcess | null = null;
  private pasdPid: number | null = null;
  private driverProc: ChildProcess | null = null;
  private driverPid: number | null = null;
  private browser: Browser | null = null;
  private signalHandler: (() => void) | null = null;
  private cleanedUp = false;

  constructor(opts: E2EContextOptions = {}) {
    ensurePrerequisites();
    this.tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "pas-e2e-"));
    this.socketPath = path.join(this.tmpDir, "pas.sock");
    this.xdgDataHome = path.join(this.tmpDir, "xdg-data");
    this.pasDataDir = path.join(this.xdgDataHome, "pas");
    this.layoutFile = path.join(this.pasDataDir, "gui-layout.json");
    this.pasdBinOverride = opts.pasdBinOverride;
    if (opts.extraPathDir) {
      this.extraPathDirs.push(opts.extraPathDir);
    }
    fs.mkdirSync(this.pasDataDir, { recursive: true });

    this.signalHandler = () => {
      if (this.driverPid !== null) sendSignal(this.driverPid, "SIGKILL");
      if (this.pasdPid !== null) sendSignal(this.pasdPid, "SIGKILL");
    };
    process.once("SIGINT", this.signalHandler);
    process.once("SIGTERM", this.signalHandler);
  }

  createGitRepo(name: string): string {
    const repoPath = path.join(this.tmpDir, name);
    fs.mkdirSync(repoPath, { recursive: true });
    execFileSync("git", ["init", "-q"], { cwd: repoPath });
    return repoPath;
  }

  createFakeAgentAlias(binName: string): string {
    const binDir = path.join(this.tmpDir, "fake-bin");
    fs.mkdirSync(binDir, { recursive: true });
    const target = path.join(binDir, binName);
    fs.writeFileSync(target, `#!/bin/sh\nexec "${FAKE_AGENT_BIN}" "$@"\n`, {
      mode: 0o755,
    });
    if (!this.extraPathDirs.includes(binDir)) {
      this.extraPathDirs.push(binDir);
    }
    return binDir;
  }

  private baseEnv(): NodeJS.ProcessEnv {
    const currentPath = process.env.PATH ?? "";
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      PAS_SOCKET: this.socketPath,
      PAS_BIN: PAS_CLI_BIN,
      XDG_DATA_HOME: this.xdgDataHome,
      PATH:
        this.extraPathDirs.length > 0
          ? `${this.extraPathDirs.join(":")}:${currentPath}`
          : currentPath,
    };
    if (this.pasdBinOverride) {
      env.PAS_PASD_BIN = this.pasdBinOverride;
    } else {
      delete env.PAS_PASD_BIN;
    }
    return env;
  }

  async startPasd(): Promise<number> {
    if (this.pasdPid !== null && isPidAlive(this.pasdPid)) {
      return this.pasdPid;
    }
    const proc = spawn(
      PASD_BIN,
      ["--socket", this.socketPath, "--data-dir", this.pasDataDir],
      {
        env: this.baseEnv(),
        detached: true,
        stdio: "ignore",
      },
    );
    if (!proc.pid) {
      throw new Error("No se obtuvo PID al iniciar pasd.");
    }
    proc.unref();
    this.pasdProc = proc;
    this.pasdPid = proc.pid;
    await waitForUnixSocket(this.socketPath);
    return proc.pid;
  }

  async killPasd(): Promise<void> {
    if (this.pasdPid === null) return;
    const pid = this.pasdPid;
    this.pasdPid = null;
    this.pasdProc = null;
    await terminateProcessByPid(pid);
  }

  async openApp(): Promise<Browser> {
    if (this.browser) {
      await this.closeApp();
    }
    const driverPort = await getFreePort();
    const nativePort = await getFreePort();

    const driver = spawn(
      TAURI_DRIVER_BIN,
      [
        "--port",
        String(driverPort),
        "--native-port",
        String(nativePort),
        "--native-driver",
        NATIVE_DRIVER_BIN,
      ],
      {
        env: this.baseEnv(),
        detached: true,
        stdio: "ignore",
      },
    );
    if (!driver.pid) {
      throw new Error("No se obtuvo PID al iniciar tauri-driver.");
    }
    driver.unref();
    this.driverProc = driver;
    this.driverPid = driver.pid;

    await waitForTcpPort(driverPort);

    const browser = await remote({
      hostname: "127.0.0.1",
      port: driverPort,
      path: "/",
      logLevel: "error",
      connectionRetryTimeout: 30_000,
      connectionRetryCount: 3,
      capabilities: {
        "tauri:options": {
          application: PAS_GUI_BIN,
        },
      } as Record<string, unknown>,
    });

    this.browser = browser;
    await browser.$(".pas-shell").waitForExist({ timeout: 15_000 });
    await browser.$(".pas-statusbar").waitForExist({ timeout: 15_000 });
    await browser.waitUntil(
      async () => (await readTextContent(browser, ".pas-statusbar")).includes("Conectado a pasd"),
      { timeout: 15_000, timeoutMsg: "pas-gui no mostró 'Conectado a pasd' al iniciar." },
    );
    return browser;
  }

  async closeApp(): Promise<void> {
    const browser = this.browser;
    this.browser = null;
    if (browser) {
      try {
        await browser.deleteSession();
      } catch {
        // Si la sesión ya cerró, seguimos con la limpieza por PID.
      }
    }
    if (this.driverPid !== null) {
      const pid = this.driverPid;
      this.driverPid = null;
      this.driverProc = null;
      await terminateProcessByPid(pid);
    }
  }

  async cleanup(): Promise<void> {
    if (this.cleanedUp) return;
    this.cleanedUp = true;

    if (this.signalHandler) {
      process.off("SIGINT", this.signalHandler);
      process.off("SIGTERM", this.signalHandler);
      this.signalHandler = null;
    }

    await this.closeApp();
    await this.killPasd();

    // Evitar referencias colgantes a los objetos ChildProcess.
    this.driverProc = null;
    this.pasdProc = null;

    fs.rmSync(this.tmpDir, { recursive: true, force: true });
  }
}

async function waitForDialogClosed(browser: Browser): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
    { timeout: 10_000, interval: 50, timeoutMsg: "El diálogo no se cerró a tiempo." },
  );
}

export async function addProjectViaUi(browser: Browser, repoPath: string): Promise<void> {
  await browser.$(".pas-add-project").click();
  const pathInput = await browser.$("#ap-path");
  await pathInput.waitForDisplayed({ timeout: 10_000 });
  await pathInput.setValue(repoPath);
  await browser.$('.pas-dialog-card button[type="submit"]').click();
  await waitForDialogClosed(browser);
}

export async function openFakeAgentSessionViaUi(
  browser: Browser,
  opts: { splitRightFromPane?: number } = {},
): Promise<void> {
  if (opts.splitRightFromPane !== undefined) {
    const panes = await browser.$$(".pas-pane");
    const targetPane = panes[opts.splitRightFromPane];
    if (!targetPane) {
      throw new Error(`No existe el pane ${opts.splitRightFromPane} para hacer split.`);
    }
    await targetPane.$('button[title*="Split a la derecha"]').click();
  } else {
    await browser.$(".pas-new-btn").click();
  }

  await browser.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });

  const radios = await browser.$$('.pas-dialog-card button[role="radio"]');
  let clickedCustom = false;
  for (const radio of radios) {
    const label = (await radio.getText()).trim();
    if (label.startsWith("Comando")) {
      await radio.click();
      clickedCustom = true;
      break;
    }
  }
  if (!clickedCustom) {
    throw new Error("No se encontró el botón 'Comando…' en el diálogo de nueva sesión.");
  }

  const cmdInput = await browser.$("#ns-cmd");
  await cmdInput.waitForDisplayed({ timeout: 10_000 });
  await cmdInput.setValue(FAKE_AGENT_BIN);

  await browser.$('.pas-dialog-card button[type="submit"]').click();
  await waitForDialogClosed(browser);
}

export async function readTextContent(browser: Browser, selector: string): Promise<string> {
  return browser.execute((sel: string) => {
    return document.querySelector(sel)?.textContent ?? "";
  }, selector);
}

export async function readPaneBuffer(browser: Browser, paneIndex: number): Promise<string> {
  return browser.execute((idx: number) => {
    const hosts = document.querySelectorAll(".pas-pane .pas-terminal-host");
    const host = hosts[idx] as
      | (HTMLDivElement & { __pasReadBuffer?: () => string })
      | undefined;
    return host?.__pasReadBuffer ? host.__pasReadBuffer() : "";
  }, paneIndex);
}

export async function waitForPaneText(
  browser: Browser,
  paneIndex: number,
  expected: string,
  timeoutMs = 15_000,
): Promise<string> {
  let latest = "";
  try {
    await browser.waitUntil(
      async () => {
        latest = await readPaneBuffer(browser, paneIndex);
        return latest.includes(expected);
      },
      {
        timeout: timeoutMs,
        interval: 100,
      },
    );
  } catch {
    throw new Error(
      `El pane ${paneIndex} no mostró ${JSON.stringify(expected)} en ${timeoutMs} ms. Contenido actual:\n${latest.trim()}`,
    );
  }
  return latest;
}

export async function typeLineInPane(
  browser: Browser,
  paneIndex: number,
  line: string,
): Promise<void> {
  const panes = await browser.$$(".pas-pane");
  const pane = panes[paneIndex];
  if (!pane) {
    throw new Error(`No existe el pane ${paneIndex} para escribir.`);
  }
  await pane.$(".pas-terminal-host").click();
  await browser.waitUntil(
    async () =>
      browser.execute((idx: number) => {
        const target = document.querySelectorAll(".pas-pane")[idx];
        const active = document.activeElement;
        return Boolean(target && active && target.contains(active));
      }, paneIndex),
    { timeout: 5_000, interval: 50 },
  );
  await browser.keys([...line.split(""), "\uE007"]);
}
