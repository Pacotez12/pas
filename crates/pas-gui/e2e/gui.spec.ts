import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  E2EContext,
  PAS_CLI_BIN,
  addProjectViaUi,
  openFakeAgentSessionViaUi,
  readTextContent,
  typeLineInPane,
  waitForPaneText,
} from "./helpers";

describe("pas-gui e2e", () => {
  it("agrega un proyecto, abre fake-agent, escribe echo hola y lo muestra", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = ctx.createGitRepo("proyecto-hola");
      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);

      await browser.waitUntil(
        async () => (await readTextContent(browser, ".pas-sidebar")).includes("proyecto-hola"),
        { timeout: 10_000, timeoutMsg: "El proyecto no apareció en la sidebar." },
      );

      await openFakeAgentSessionViaUi(browser);
      await waitForPaneText(browser, 0, "fake-agent listo");

      await typeLineInPane(browser, 0, "echo hola");
      const content = await waitForPaneText(browser, 0, "\nhola");
      expect(content).toContain("hola");
    } finally {
      await ctx.cleanup();
    }
  });

  it("hace split a la derecha con dos terminales vivas", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = ctx.createGitRepo("proyecto-split");
      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);
      await openFakeAgentSessionViaUi(browser);
      await waitForPaneText(browser, 0, "fake-agent listo");

      await openFakeAgentSessionViaUi(browser, { splitRightFromPane: 0 });

      await browser.waitUntil(
        async () => (await browser.$$(".pas-split.is-row .pas-pane")).length === 2,
        { timeout: 10_000, timeoutMsg: "No se crearon dos paneles en split horizontal." },
      );

      await waitForPaneText(browser, 1, "fake-agent listo");

      await typeLineInPane(browser, 0, "echo terminal-uno");
      await waitForPaneText(browser, 0, "\nterminal-uno");

      await typeLineInPane(browser, 1, "echo terminal-dos");
      await waitForPaneText(browser, 1, "\nterminal-dos");
    } finally {
      await ctx.cleanup();
    }
  });

  it("conserva el layout y el contenido al cerrar y reabrir la app", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = ctx.createGitRepo("proyecto-persiste");
      await ctx.startPasd();
      const browser1 = await ctx.openApp();

      await addProjectViaUi(browser1, repoPath);
      await openFakeAgentSessionViaUi(browser1);
      await waitForPaneText(browser1, 0, "fake-agent listo");
      await typeLineInPane(browser1, 0, "echo marca-izquierda");
      await waitForPaneText(browser1, 0, "\nmarca-izquierda");

      await openFakeAgentSessionViaUi(browser1, { splitRightFromPane: 0 });
      await browser1.waitUntil(
        async () => (await browser1.$$(".pas-split.is-row .pas-pane")).length === 2,
        { timeout: 10_000 },
      );
      await waitForPaneText(browser1, 1, "fake-agent listo");
      await typeLineInPane(browser1, 1, "echo marca-derecha");
      await waitForPaneText(browser1, 1, "\nmarca-derecha");

      // Esperar que el debounce de 150 ms persista el split en gui-layout.json.
      await browser1.waitUntil(
        async () => {
          if (!fs.existsSync(ctx.layoutFile)) return false;
          const raw = fs.readFileSync(ctx.layoutFile, "utf8");
          return raw.includes('"kind":"split"');
        },
        { timeout: 5_000, timeoutMsg: "No se persistió el layout con split en gui-layout.json." },
      );

      // Cerrar solo la ventana/app; pasd sigue corriendo.
      await ctx.closeApp();

      const browser2 = await ctx.openApp();

      await browser2.waitUntil(
        async () => (await browser2.$$(".pas-split.is-row .pas-pane")).length === 2,
        {
          timeout: 10_000,
          timeoutMsg: "Al reabrir la app no se restauró el split de dos paneles.",
        },
      );

      await waitForPaneText(browser2, 0, "marca-izquierda");
      await waitForPaneText(browser2, 1, "marca-derecha");

      // Confirmar que las terminales siguen vivas tras reabrir.
      await typeLineInPane(browser2, 1, "echo sigue-vivo");
      await waitForPaneText(browser2, 1, "\nsigue-vivo");
    } finally {
      await ctx.cleanup();
    }
  });

  it("muestra desconectado al matar pasd y se reconecta al volver", async () => {
    const ctx = new E2EContext({
      pasdBinOverride: "/ruta/inexistente/pasd-deshabilitado-para-e2e",
    });
    try {
      const repoPath = ctx.createGitRepo("proyecto-reconecta");
      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);
      await openFakeAgentSessionViaUi(browser);
      await waitForPaneText(browser, 0, "fake-agent listo");

      await browser.waitUntil(
        async () => {
          const status = await browser.$(".pas-statusbar").getText();
          return status.includes("Conectado a pasd") && status.includes("1 sesión");
        },
        { timeout: 10_000 },
      );

      // Matar pasd por su PID guardado; con PAS_PASD_BIN inexistente no puede autoarrancar.
      await ctx.killPasd();

      await browser.waitUntil(
        async () => {
          const bannerExists = await browser.$(".pas-banner").isExisting();
          const status = await browser.$(".pas-statusbar").getText();
          return bannerExists && status.includes("Desconectado");
        },
        {
          timeout: 10_000,
          timeoutMsg: "No apareció el aviso de desconexión tras matar pasd.",
        },
      );

      const bannerText = await browser.$(".pas-banner").getText();
      expect(bannerText).toContain("Se perdió la conexión con pasd");

      // Volver a levantar pasd en el mismo socket y data-dir: la app reconecta sola y actualiza la sidebar.
      await ctx.startPasd();

      await browser.waitUntil(
        async () => {
          const bannerExists = await browser.$(".pas-banner").isExisting();
          const status = await browser.$(".pas-statusbar").getText();
          return (
            !bannerExists &&
            status.includes("Conectado a pasd") &&
            status.includes("0 sesiones")
          );
        },
        {
          timeout: 15_000,
          timeoutMsg: "La app no reconectó ni actualizó el estado al volver pasd.",
        },
      );

      const sidebarText = await readTextContent(browser, ".pas-sidebar");
      expect(sidebarText).toContain("proyecto-reconecta");
    } finally {
      await ctx.cleanup();
    }
  });

  it("muestra la franja al terminar una sesión y permite reabrirla en el mismo pane", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = ctx.createGitRepo("proyecto-reabrir");
      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);
      await openFakeAgentSessionViaUi(browser);
      await waitForPaneText(browser, 0, "fake-agent listo");

      await typeLineInPane(browser, 0, "echo antes-de-salir");
      await waitForPaneText(browser, 0, "\nantes-de-salir");

      const oldSessionId = await browser.$(".pas-pane").getAttribute("data-session-id");
      expect(oldSessionId).toBeTruthy();

      await typeLineInPane(browser, 0, "exit 0");

      try {
        await browser.waitUntil(
          async () => {
            const text = await readTextContent(browser, ".pas-pane-exited-bar");
            return text.includes("La sesión terminó (código 0).");
          },
          { timeout: 10_000, interval: 100 },
        );
      } catch {
        const barText = await readTextContent(browser, ".pas-pane-exited-bar");
        const sidebarText = await readTextContent(browser, ".pas-sidebar");
        const buf = await browser.execute(() => {
          const host = document.querySelector(".pas-pane .pas-terminal-host") as
            | (HTMLDivElement & { __pasReadBuffer?: () => string })
            | null;
          return host?.__pasReadBuffer?.() ?? "";
        });
        throw new Error(
          `No apareció la franja de sesión terminada con código 0. bar=${JSON.stringify(barText)} sidebar=${JSON.stringify(sidebarText)} buf=${JSON.stringify(buf.trim())}`,
        );
      }

      // El pane conserva lo último que mostró antes de salir.
      await waitForPaneText(browser, 0, "antes-de-salir");

      await browser.$(".pas-pane-exited-bar button.is-primary").click();

      await browser.waitUntil(
        async () => {
          const barExists = await browser.$(".pas-pane-exited-bar").isExisting();
          const currentId = await browser.$(".pas-pane").getAttribute("data-session-id");
          return !barExists && Boolean(currentId) && currentId !== oldSessionId;
        },
        {
          timeout: 10_000,
          timeoutMsg: "Reabrir no reemplazó la sesión terminada en el mismo pane.",
        },
      );

      await waitForPaneText(browser, 0, "fake-agent listo");
      await typeLineInPane(browser, 0, "echo despues-de-reabrir");
      await waitForPaneText(browser, 0, "\ndespues-de-reabrir");

      const status = await browser.$(".pas-statusbar").getText();
      expect(status).toContain("1 sesión");
    } finally {
      await ctx.cleanup();
    }
  });

  it("crea una cuenta en tempdir, abre sesión con esa cuenta y muestra punto naranja al simular hook", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = ctx.createGitRepo("proyecto-cuenta");
      const accountDir = path.join(ctx.tmpDir, "fake-claude-account");
      fs.mkdirSync(accountDir, { recursive: true });
      ctx.createFakeAgentAlias("claude");

      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);

      // Abrir pantalla Cuentas desde el engranaje.
      await browser.$('button[aria-label="Cuentas"]').click();
      await browser.$(".pas-accounts-view").waitForDisplayed({ timeout: 10_000 });

      // Agregar cuenta apuntando al tempdir.
      await browser.$(".pas-accounts-footer button").click();
      const nameInput = await browser.$("#acc-name");
      await nameInput.waitForDisplayed({ timeout: 10_000 });
      await nameInput.setValue("Personal");

      const dirRadios = await browser.$$('.pas-dialog-card button[role="radio"]');
      for (const radio of dirRadios) {
        if ((await radio.getText()).includes("Usar uno existente")) {
          await radio.click();
          break;
        }
      }
      const dirInput = await browser.$("#acc-dir");
      await dirInput.waitForDisplayed({ timeout: 10_000 });
      await dirInput.setValue(accountDir);

      await browser.$('.pas-dialog-card button[type="submit"]').click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      // Verificar cuenta en la pantalla Cuentas y pastilla de cuota en la barra inferior.
      await browser.waitUntil(
        async () =>
          (await readTextContent(browser, ".pas-accounts-view")).includes("Personal") &&
          (await readTextContent(browser, ".pas-statusbar")).includes("Claude · Personal") &&
          (await readTextContent(browser, ".pas-statusbar")).includes("sin datos"),
        { timeout: 10_000, timeoutMsg: "La cuenta o su pastilla de cuota no aparecieron." },
      );

      // Verificar diálogo de confirmación de hooks con la ruta exacta (sin instalar fuera del tempdir).
      const installButtons = await browser.$$(".pas-acc-card button.is-primary");
      expect(installButtons.length).toBe(2);
      await installButtons[0].click();
      await browser.$(".pas-hooks-path").waitForDisplayed({ timeout: 10_000 });
      const hooksPathText = await readTextContent(browser, ".pas-hooks-path");
      expect(hooksPathText).toBe(path.join(accountDir, "settings.json"));
      await browser.$(".pas-dialog-actions button:first-child").click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      // Verificar que la fila de agy (Predeterminada) tenga botón Instalar hooks.
      const agyInstallBtn = await browser.$(
        '[data-agent-group="agy"] .pas-acc-card button.is-primary',
      );
      expect(await agyInstallBtn.isExisting()).toBe(true);
      expect(await agyInstallBtn.getText()).toContain("Instalar hooks");

      // Volver al workspace.
      await browser.$(".pas-accounts-header button").click();
      await browser.$(".pas-sidebar").waitForDisplayed({ timeout: 10_000 });

      // Abrir sesión de Claude Code con la cuenta Personal.
      await browser.$(".pas-new-btn").click();
      await browser.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });
      const accountComboText = await readTextContent(
        browser,
        'button[aria-labelledby="ns-account-label"]',
      );
      expect(accountComboText).toContain("Personal");

      await browser.$('.pas-dialog-card button[type="submit"]').click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      await waitForPaneText(browser, 0, "fake-agent listo");

      const sessionId = await browser.$(".pas-pane").getAttribute("data-session-id");
      expect(sessionId).toBeTruthy();

      // Simular hook PermissionRequest con pas hook y PAS_SESSION_ID.
      execFileSync(PAS_CLI_BIN, ["hook", "claude", "PermissionRequest"], {
        env: {
          ...process.env,
          PAS_SESSION_ID: sessionId,
          PAS_SOCKET: ctx.socketPath,
        },
        input: "{}",
        stdio: ["pipe", "pipe", "pipe"],
      });

      // Confirmar punto naranja en sidebar y pestaña + texto "Personal · esperando tu respuesta".
      await browser.waitUntil(
        async () => {
          const sidebarHasWaitDot = await browser
            .$(`.pas-sidebar .pas-ses[data-session-id="${sessionId}"] .pas-dot.is-wait`)
            .isExisting();
          const tabHasWaitDot = await browser
            .$(".pas-tabs .pas-tab.is-active .pas-dot.is-wait")
            .isExisting();
          const metaText = await readTextContent(
            browser,
            `.pas-sidebar .pas-ses[data-session-id="${sessionId}"] .pas-ses-meta`,
          );
          return (
            sidebarHasWaitDot &&
            tabHasWaitDot &&
            metaText.includes("Personal · esperando tu respuesta")
          );
        },
        {
          timeout: 10_000,
          interval: 100,
          timeoutMsg:
            "No apareció el punto naranja ni el texto 'Personal · esperando tu respuesta'.",
        },
      );

      // Simular UserPromptSubmit y confirmar vuelta a trabajando.
      execFileSync(PAS_CLI_BIN, ["hook", "claude", "UserPromptSubmit"], {
        env: {
          ...process.env,
          PAS_SESSION_ID: sessionId,
          PAS_SOCKET: ctx.socketPath,
        },
        input: "{}",
        stdio: ["pipe", "pipe", "pipe"],
      });

      await browser.waitUntil(
        async () =>
          browser
            .$(`.pas-sidebar .pas-ses[data-session-id="${sessionId}"] .pas-dot.is-ok`)
            .isExisting(),
        { timeout: 10_000, interval: 100 },
      );
    } finally {
      await ctx.cleanup();
    }
  });
});
