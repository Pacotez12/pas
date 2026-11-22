import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  E2EContext,
  FAKE_AGENT_BIN,
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
      await browser.$(".pas-accounts-footer button:first-child").click();
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

  it("abre un chat con fake-acp-agent, aprueba permiso, muestra diff y visual, y conserva todo al reabrir", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = ctx.createGitRepo("proyecto-chat");
      ctx.createFakeAcpAdapterAlias("claude-agent-acp", [
        { type: "text", chunks: ["Busco ", "el archivo."] },
        { type: "tool", title: "Leyó src/export.ts", status: "completed" },
        {
          type: "permission",
          title: "Claude quiere ejecutar un comando",
          detail: "npm test",
          after_allow: "Permiso aprobado, aplico el cambio.",
        },
        {
          type: "diff",
          path: "src/export.ts",
          old_text: "export const v = 1;",
          new_text: "export const v = 2;\nexport const csv = true;",
        },
        {
          type: "visual",
          file: "mockup-exportar.html",
          html: "<html><body><h1>Vista previa CSV</h1></body></html>",
          message: "Te dejo el visual:",
        },
      ]);

      await ctx.startPasd();
      const browser1 = await ctx.openApp();

      await addProjectViaUi(browser1, repoPath);

      // Abrir nueva sesión en modo Chat.
      await browser1.$(".pas-new-btn").click();
      await browser1.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });

      const modeRadios = await browser1.$$(
        '.pas-dialog-card [aria-label="Modo"] button[role="radio"]',
      );
      let clickedChat = false;
      for (const radio of modeRadios) {
        if ((await radio.getText()).trim() === "Chat") {
          await radio.click();
          clickedChat = true;
          break;
        }
      }
      expect(clickedChat).toBe(true);

      const submitBtn = await browser1.$('.pas-dialog-card button[type="submit"]');
      expect((await submitBtn.getText()).trim()).toBe("Abrir chat");
      await submitBtn.click();

      await browser1.waitUntil(
        async () =>
          browser1.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      // Confirmar que se abrió un ChatPane y que el botón Abrir en terminal aparece para Claude.
      await browser1.$(".pas-chat-pane").waitForDisplayed({ timeout: 10_000 });
      await browser1.waitUntil(
        async () =>
          (await readTextContent(browser1, ".pas-chat-pane .pas-pane-header")).includes(
            "Abrir en terminal",
          ),
        { timeout: 10_000, timeoutMsg: "No apareció 'Abrir en terminal' en el chat de Claude." },
      );

      // Enviar mensaje con Enter desde el composer.
      const composer = await browser1.$(".pas-chat-ta");
      await composer.click();
      await composer.setValue("Agregá exportación CSV");
      await browser1.keys(["\uE007"]);

      // Esperar respuesta en streaming, herramienta y tarjeta de permiso (con estado waiting).
      await browser1.waitUntil(
        async () => {
          const feedText = await readTextContent(browser1, ".pas-chat-feed");
          const permExists = await browser1.$(".pas-chat-perm").isExisting();
          const waitDot = await browser1.$(".pas-tab.is-active .pas-dot.is-wait").isExisting();
          return (
            feedText.includes("Agregá exportación CSV") &&
            feedText.includes("Busco el archivo.") &&
            feedText.includes("Leyó src/export.ts") &&
            permExists &&
            waitDot
          );
        },
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "No llegaron los mensajes iniciales o la tarjeta de permiso en el chat.",
        },
      );

      // Aprobar permiso con el botón Permitir.
      await browser1.$(".pas-chat-perm-actions button.is-primary").click();

      // Esperar permiso resuelto, diff y visual en iframe aislado.
      await browser1.waitUntil(
        async () => {
          const permResolved =
            (await browser1.$(".pas-chat-perm").getAttribute("data-resolution")) === "allow";
          const feedText = await readTextContent(browser1, ".pas-chat-feed");
          const diffExists = await browser1.$(".pas-chat-diff").isExisting();
          const frameExists = await browser1.$(".pas-chat-vis-frame").isExisting();
          return (
            permResolved &&
            feedText.includes("Permiso aprobado, aplico el cambio.") &&
            diffExists &&
            frameExists
          );
        },
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "No aparecieron el permiso resuelto, el diff y el visual tras aprobar.",
        },
      );

      const sandboxAttr = await browser1.$(".pas-chat-vis-frame").getAttribute("sandbox");
      expect(sandboxAttr).toBe("allow-scripts");
      const visHeadText = await readTextContent(browser1, ".pas-chat-vis-head");
      expect(visHeadText).toContain("mockup-exportar.html");
      expect(visHeadText).toContain("Abrir en el navegador");

      // Verificar títulos de herramientas (nunca el id crudo "tc-2") y orden exacto en el feed.
      const feedSnapshot = await browser1.execute(() => {
        const feed = document.querySelector(".pas-chat-feed");
        const children = feed ? Array.from(feed.children) : [];
        return children.map((el) => ({
          cls: el.className,
          text: (el.textContent ?? "").trim(),
        }));
      });

      const toolTitles = await browser1.execute(() =>
        Array.from(document.querySelectorAll(".pas-chat-tool-title")).map((el) =>
          (el.textContent ?? "").trim(),
        ),
      );
      expect(toolTitles).not.toContain("tc-2");
      expect(toolTitles).toContain("Claude quiere ejecutar un comando");

      const idxPerm = feedSnapshot.findIndex(
        (c) => c.cls.includes("pas-chat-perm") && c.text.includes("Permitido"),
      );
      const idxAfterAllow = feedSnapshot.findIndex(
        (c) =>
          c.cls.includes("pas-chat-ai") &&
          c.text.includes("Permiso aprobado, aplico el cambio."),
      );
      const idxEditTool = feedSnapshot.findIndex(
        (c) => c.cls.includes("pas-chat-tool") && c.text.includes("Editó src/export.ts"),
      );
      const idxDiff = feedSnapshot.findIndex(
        (c) => c.cls.includes("pas-chat-diff") && c.text.includes("src/export.ts"),
      );
      const idxVisText = feedSnapshot.findIndex(
        (c) => c.cls.includes("pas-chat-ai") && c.text.includes("Te dejo el visual:"),
      );
      const idxVisual = feedSnapshot.findIndex(
        (c) => c.cls.includes("pas-chat-vis") && c.text.includes("mockup-exportar.html"),
      );

      expect(idxPerm).toBeGreaterThanOrEqual(0);
      expect(idxAfterAllow).toBeGreaterThan(idxPerm);
      expect(idxEditTool).toBeGreaterThan(idxAfterAllow);
      expect(idxDiff).toBeGreaterThan(idxEditTool);
      expect(idxVisText).toBeGreaterThan(idxDiff);
      expect(idxVisual).toBeGreaterThan(idxVisText);

      // Esperar que el layout se guarde en gui-layout.json.
      await browser1.waitUntil(
        async () => fs.existsSync(ctx.layoutFile) && fs.readFileSync(ctx.layoutFile, "utf8").includes('"tabs"'),
        { timeout: 5_000 },
      );

      // Cerrar y reabrir la app (pasd sigue vivo) y verificar que todo reaparece sin duplicados.
      await ctx.closeApp();
      const browser2 = await ctx.openApp();

      await browser2.$(".pas-chat-pane").waitForDisplayed({ timeout: 10_000 });
      await browser2.waitUntil(
        async () => {
          const feedText = await readTextContent(browser2, ".pas-chat-feed");
          const permResolved =
            (await browser2.$(".pas-chat-perm").getAttribute("data-resolution")) === "allow";
          const diffCount = (await browser2.$$(".pas-chat-diff")).length;
          const visCount = (await browser2.$$(".pas-chat-vis-frame")).length;
          const userCount = (await browser2.$$(".pas-chat-me")).length;
          return (
            feedText.includes("Agregá exportación CSV") &&
            feedText.includes("Busco el archivo.") &&
            feedText.includes("Permiso aprobado, aplico el cambio.") &&
            permResolved &&
            diffCount === 1 &&
            visCount === 1 &&
            userCount === 1
          );
        },
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "Al reabrir la app el historial del chat no se restauró igual y sin duplicados.",
        },
      );
    } finally {
      await ctx.cleanup();
    }
  });

  it("abre sesión en worktree, commitea, termina, mergea y borra el worktree", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = path.join(ctx.tmpDir, "proyecto-wt-merge");
      fs.mkdirSync(repoPath, { recursive: true });
      execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repoPath });
      execFileSync("git", ["config", "user.name", "Test"], { cwd: repoPath });
      execFileSync("git", ["config", "user.email", "test@example.com"], { cwd: repoPath });
      fs.writeFileSync(path.join(repoPath, "README.md"), "inicial\n", "utf8");
      execFileSync("git", ["add", "README.md"], { cwd: repoPath });
      execFileSync("git", ["commit", "-q", "-m", "init"], { cwd: repoPath });
      execFileSync("git", ["branch", "develop"], { cwd: repoPath });

      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);

      await browser.$(".pas-new-btn").click();
      await browser.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });

      const radios = await browser.$$('.pas-dialog-card button[role="radio"]');
      for (const radio of radios) {
        const label = (await radio.getText()).trim();
        if (label.startsWith("Comando")) {
          await radio.click();
          break;
        }
      }
      const cmdInput = await browser.$("#ns-cmd");
      await cmdInput.waitForDisplayed({ timeout: 10_000 });
      await cmdInput.setValue(FAKE_AGENT_BIN);

      const folderRadios = await browser.$$('.pas-dialog-card button[role="radio"]');
      for (const radio of folderRadios) {
        const label = (await radio.getText()).trim();
        if (label === "Worktree nuevo") {
          await radio.click();
          break;
        }
      }

      const branchInput = await browser.$("#ns-wt-branch");
      await branchInput.waitForDisplayed({ timeout: 10_000 });
      await browser.waitUntil(
        async () => (await branchInput.getValue()).trim().length > 0,
        { timeout: 10_000, timeoutMsg: "No se sugirió el nombre de rama del worktree." },
      );
      const baseTrigger = await browser.$("#ns-wt-base");
      await browser.waitUntil(
        async () => (await readTextContent(browser, "#ns-wt-base")).trim() === "main",
        { timeout: 10_000, timeoutMsg: "El selector Desde no preseleccionó main." },
      );
      await baseTrigger.click();
      await browser.waitUntil(
        async () => {
          const menuText = await readTextContent(browser, ".pas-wt-col-base .pas-select-menu");
          return menuText.includes("main") && menuText.includes("develop");
        },
        { timeout: 5_000, timeoutMsg: "El desplegable Desde no listó las ramas locales." },
      );
      await baseTrigger.click();
      await branchInput.setValue("pas/e2e-merge");

      await browser.$('.pas-dialog-card button[type="submit"]').click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      await waitForPaneText(browser, 0, "fake-agent listo");

      await browser.waitUntil(
        async () => {
          const sideBr = await readTextContent(browser, ".pas-sidebar .pas-br");
          const paneBr = await readTextContent(browser, ".pas-pane-header .pas-br");
          return sideBr.includes("pas/e2e-merge") && paneBr.includes("pas/e2e-merge");
        },
        {
          timeout: 10_000,
          timeoutMsg: "La rama del worktree no apareció en la sidebar y en el encabezado del panel.",
        },
      );

      const wtPath = path.join(ctx.pasDataDir, "worktrees", "proyecto-wt-merge", "pas-e2e-merge");
      expect(fs.existsSync(wtPath)).toBe(true);

      fs.writeFileSync(path.join(wtPath, "feature.txt"), "hecho en worktree\n", "utf8");
      execFileSync("git", ["add", "feature.txt"], { cwd: wtPath });
      execFileSync("git", ["commit", "-q", "-m", "feat: e2e"], { cwd: wtPath });

      await typeLineInPane(browser, 0, "exit 0");

      await browser.waitUntil(
        async () => {
          const strip = await readTextContent(browser, ".pas-pane-exited-bar.is-worktree");
          return (
            strip.includes("1 commit que no está en main") &&
            strip.includes("Mergear y borrar")
          );
        },
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "No apareció la franja de worktree terminado con el resumen de la rama.",
        },
      );

      const stripButtons = await browser.$$(".pas-pane-exited-bar.is-worktree button");
      let clickedMergeAndRemove = false;
      for (const btn of stripButtons) {
        if ((await btn.getText()).trim() === "Mergear y borrar") {
          await btn.click();
          clickedMergeAndRemove = true;
          break;
        }
      }
      expect(clickedMergeAndRemove).toBe(true);

      await browser.waitUntil(
        async () =>
          !fs.existsSync(wtPath) &&
          fs.existsSync(path.join(repoPath, "feature.txt")),
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "Mergear y borrar no mergeó el archivo al repo principal ni borró el worktree.",
        },
      );

      const branches = execFileSync("git", ["branch", "--list", "pas/e2e-merge"], {
        cwd: repoPath,
        encoding: "utf8",
      }).trim();
      expect(branches).toBe("");
    } finally {
      await ctx.cleanup();
    }
  });

  it("borrar worktree con cambios sin commitear pide confirmación con casilla obligatoria", async () => {
    const ctx = new E2EContext();
    try {
      const repoPath = path.join(ctx.tmpDir, "proyecto-wt-loss");
      fs.mkdirSync(repoPath, { recursive: true });
      execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repoPath });
      execFileSync("git", ["config", "user.name", "Test"], { cwd: repoPath });
      execFileSync("git", ["config", "user.email", "test@example.com"], { cwd: repoPath });
      fs.writeFileSync(path.join(repoPath, "README.md"), "inicial\n", "utf8");
      execFileSync("git", ["add", "README.md"], { cwd: repoPath });
      execFileSync("git", ["commit", "-q", "-m", "init"], { cwd: repoPath });
      execFileSync("git", ["branch", "develop"], { cwd: repoPath });

      await ctx.startPasd();
      const browser = await ctx.openApp();

      await addProjectViaUi(browser, repoPath);

      await browser.$(".pas-new-btn").click();
      await browser.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });

      const radios = await browser.$$('.pas-dialog-card button[role="radio"]');
      for (const radio of radios) {
        if ((await radio.getText()).trim().startsWith("Comando")) {
          await radio.click();
          break;
        }
      }
      const cmdInput = await browser.$("#ns-cmd");
      await cmdInput.waitForDisplayed({ timeout: 10_000 });
      await cmdInput.setValue(FAKE_AGENT_BIN);

      const folderRadios = await browser.$$('.pas-dialog-card button[role="radio"]');
      for (const radio of folderRadios) {
        if ((await radio.getText()).trim() === "Worktree nuevo") {
          await radio.click();
          break;
        }
      }

      const branchInput = await browser.$("#ns-wt-branch");
      await branchInput.waitForDisplayed({ timeout: 10_000 });
      await browser.waitUntil(
        async () => (await branchInput.getValue()).trim().length > 0,
        { timeout: 10_000 },
      );
      await branchInput.setValue("pas/e2e-loss");

      await browser.$('.pas-dialog-card button[type="submit"]').click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      await waitForPaneText(browser, 0, "fake-agent listo");

      const wtPath = path.join(ctx.pasDataDir, "worktrees", "proyecto-wt-loss", "pas-e2e-loss");
      expect(fs.existsSync(wtPath)).toBe(true);
      fs.writeFileSync(path.join(wtPath, "pendiente.txt"), "sin commitear\n", "utf8");

      await typeLineInPane(browser, 0, "exit 0");

      await browser.waitUntil(
        async () => {
          const strip = await readTextContent(browser, ".pas-pane-exited-bar.is-worktree");
          return (
            strip.includes("1 archivo sin commitear") &&
            strip.includes("Solo borrar")
          );
        },
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "No apareció la franja de worktree con 1 archivo sin commitear.",
        },
      );

      const stripButtons = await browser.$$(".pas-pane-exited-bar.is-worktree button");
      for (const btn of stripButtons) {
        if ((await btn.getText()).trim() === "Solo borrar") {
          const cls = (await btn.getAttribute("class")) ?? "";
          expect(cls).not.toContain("is-primary");
          await btn.click();
          break;
        }
      }

      const confirmDialog = await browser.$('[data-dialog="worktree-confirm-remove"]');
      await confirmDialog.waitForDisplayed({ timeout: 10_000 });

      const dialogText = await readTextContent(browser, '[data-dialog="worktree-confirm-remove"]');
      expect(dialogText).toContain("Borrar el worktree pas/e2e-loss");
      expect(dialogText).toContain("pendiente.txt");

      const dangerBtn = await browser.$('[data-dialog="worktree-confirm-remove"] button.is-danger');
      expect(await dangerBtn.isEnabled()).toBe(false);
      expect(fs.existsSync(wtPath)).toBe(true);

      await browser.$("#wt-confirm-loss").click();
      await browser.waitUntil(async () => dangerBtn.isEnabled(), {
        timeout: 5_000,
        timeoutMsg: "Borrar igual no se habilitó tras marcar la casilla.",
      });

      await dangerBtn.click();

      await browser.waitUntil(
        async () =>
          !fs.existsSync(wtPath) &&
          (await browser.execute(
            () => document.querySelector('[data-dialog="worktree-confirm-remove"]') === null,
          )),
        {
          timeout: 15_000,
          interval: 100,
          timeoutMsg: "El worktree no se borró tras confirmar con la casilla.",
        },
      );
    } finally {
      await ctx.cleanup();
    }
  });

  it("agrega proyecto sugerido, abre en editor falso desde menú y terminal (Ctrl+click), y salta con Ctrl+K", async () => {
    const ctx = new E2EContext();
    try {
      const discoverRoot = path.join(ctx.tmpDir, "mis-proyectos");
      const suggestedRepo = path.join(discoverRoot, "pockt-demo");
      fs.mkdirSync(path.join(suggestedRepo, "lib"), { recursive: true });
      execFileSync("git", ["init", "-q"], { cwd: suggestedRepo });
      const dartFile = path.join(suggestedRepo, "lib", "export.dart");
      fs.writeFileSync(dartFile, "void exportCsv() {}\n", "utf8");

      const { editorPath, logPath } = ctx.createFakeEditor("code");
      ctx.createFakeAgentAlias("claude");
      ctx.writeConfig(
        [
          `editor = ${JSON.stringify(editorPath)}`,
          `discover_dirs = [${JSON.stringify(discoverRoot)}]`,
        ].join("\n") + "\n",
      );

      await ctx.startPasd();
      const browser = await ctx.openApp();

      // 1. Agregar proyecto sugerido sin escribir ruta.
      await browser.$(".pas-add-project").click();
      const suggestedRow = await browser.$(`[data-discovered-path="${suggestedRepo}"]`);
      await suggestedRow.waitForDisplayed({ timeout: 10_000 });
      await suggestedRow.click();

      const submitBtn = await browser.$('.pas-dialog-card button[type="submit"]');
      expect((await submitBtn.getText()).trim()).toBe("Agregar 1 proyecto");
      await submitBtn.click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      await browser.waitUntil(
        async () => (await readTextContent(browser, ".pas-sidebar")).includes("pockt-demo"),
        { timeout: 10_000, timeoutMsg: "El proyecto sugerido no apareció en la sidebar." },
      );

      // 2. Abrir en el editor desde el menú contextual del proyecto.
      const famBtn = await browser.$(".pas-sidebar .pas-fam");
      await famBtn.click({ button: "right" });
      await browser.$(".pas-project-menu").waitForDisplayed({ timeout: 10_000 });
      const menuItems = await browser.$$(".pas-project-menu .pas-menu-item");
      let clickedOpenEditor = false;
      for (const item of menuItems) {
        if ((await item.getText()).includes("Abrir en el editor")) {
          await item.click();
          clickedOpenEditor = true;
          break;
        }
      }
      expect(clickedOpenEditor).toBe(true);

      const canonRepo = fs.realpathSync(suggestedRepo);
      await browser.waitUntil(
        async () =>
          fs.existsSync(logPath) &&
          fs.readFileSync(logPath, "utf8").includes(canonRepo),
        {
          timeout: 10_000,
          interval: 100,
          timeoutMsg: "El lanzador falso no registró la apertura del proyecto en el editor.",
        },
      );

      // 3. Abrir sesión de terminal, imprimir lib/export.dart:42 y hacer Ctrl+click.
      await browser.$(".pas-new-btn").click();
      await browser.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });
      await browser.$('.pas-dialog-card button[type="submit"]').click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );
      await waitForPaneText(browser, 0, "fake-agent listo");
      const firstSessionId = await browser.$(".pas-pane").getAttribute("data-session-id");
      expect(firstSessionId).toBeTruthy();

      // Enviar la línea vía pas CLI (evita mapeo de teclado XKB en WebKitWebDriver para '/' y ':').
      execFileSync(PAS_CLI_BIN, ["send", firstSessionId!, "echo lib/export.dart:42"], {
        env: { ...process.env, PAS_SOCKET: ctx.socketPath },
        stdio: ["ignore", "pipe", "pipe"],
      });
      await waitForPaneText(browser, 0, "\nlib/export.dart:42");

      // Ubicar las coordenadas de lib/export.dart:42 en la grilla de xterm y simular Ctrl+hover.
      const coords = await browser.execute(() => {
        const pane = document.querySelector(".pas-pane");
        const screen = pane?.querySelector(".xterm-screen") as HTMLElement | null;
        const dimsText = pane?.querySelector(".pas-pane-dims")?.textContent ?? "";
        const host = pane?.querySelector(".pas-terminal-host") as
          | (HTMLDivElement & { __pasReadBuffer?: () => string })
          | null;
        if (!screen || !host?.__pasReadBuffer) return null;
        const [colsStr, rowsStr] = dimsText.split("×");
        const cols = Number.parseInt(colsStr, 10);
        const rows = Number.parseInt(rowsStr, 10);
        if (!cols || !rows) return null;
        const lines = host.__pasReadBuffer().split("\n");
        let targetRow = -1;
        let targetCol = -1;
        for (let r = lines.length - 1; r >= 0; r -= 1) {
          const idx = lines[r].indexOf("lib/export.dart:42");
          if (idx !== -1 && !lines[r].includes("echo ")) {
            targetRow = r;
            targetCol = idx;
            break;
          }
        }
        if (targetRow === -1) return null;
        const rect = screen.getBoundingClientRect();
        const cellW = rect.width / cols;
        const cellH = rect.height / rows;
        return {
          x: rect.left + (targetCol + 4) * cellW,
          y: rect.top + (targetRow + 0.5) * cellH,
        };
      });
      expect(coords).not.toBeNull();

      await browser.execute((pt: { x: number; y: number }) => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "Control", ctrlKey: true, bubbles: true }));
        const target =
          (document.elementFromPoint(pt.x, pt.y) as HTMLElement | null) ??
          document.querySelector(".pas-pane .xterm-screen");
        target?.dispatchEvent(
          new MouseEvent("mousemove", {
            clientX: pt.x,
            clientY: pt.y,
            ctrlKey: true,
            bubbles: true,
          }),
        );
      }, coords!);

      await browser.waitUntil(
        async () => browser.$(".pas-term-tip").isExisting(),
        {
          timeout: 10_000,
          interval: 100,
          timeoutMsg: "No apareció el tooltip de Ctrl+click al pasar sobre la ruta de la terminal.",
        },
      );

      await browser.execute((pt: { x: number; y: number }) => {
        const target =
          (document.elementFromPoint(pt.x, pt.y) as HTMLElement | null) ??
          document.querySelector(".pas-pane .xterm-screen");
        target?.dispatchEvent(
          new MouseEvent("mousedown", {
            clientX: pt.x,
            clientY: pt.y,
            ctrlKey: true,
            button: 0,
            bubbles: true,
          }),
        );
        target?.dispatchEvent(
          new MouseEvent("mouseup", {
            clientX: pt.x,
            clientY: pt.y,
            ctrlKey: true,
            button: 0,
            bubbles: true,
          }),
        );
        target?.dispatchEvent(
          new MouseEvent("click", {
            clientX: pt.x,
            clientY: pt.y,
            ctrlKey: true,
            button: 0,
            bubbles: true,
          }),
        );
        window.dispatchEvent(new KeyboardEvent("keyup", { key: "Control", ctrlKey: false, bubbles: true }));
      }, coords!);

      const canonFile = fs.realpathSync(dartFile);
      await browser.waitUntil(
        async () =>
          fs.existsSync(logPath) &&
          fs.readFileSync(logPath, "utf8").includes(`-g ${canonFile}:42`),
        {
          timeout: 10_000,
          interval: 100,
          timeoutMsg: `El lanzador falso no recibió -g ${canonFile}:42 tras Ctrl+click en la terminal.`,
        },
      );

      // 4. Abrir una segunda sesión (con título distinto) y volver a la primera usando Ctrl+K.
      await browser.$(".pas-new-btn").click();
      await browser.$(".pas-dialog-card").waitForDisplayed({ timeout: 10_000 });
      const titleInput = await browser.$("#ns-title");
      await titleInput.setValue("segunda-sesion");
      await browser.$('.pas-dialog-card button[type="submit"]').click();
      await browser.waitUntil(
        async () =>
          browser.execute(() => document.querySelector(".pas-dialog-backdrop") === null),
        { timeout: 10_000, interval: 50 },
      );

      await browser.waitUntil(
        async () => (await browser.$$(".pas-tab")).length === 2,
        { timeout: 10_000, timeoutMsg: "No se abrió la segunda pestaña." },
      );

      // Abrir paleta con Ctrl+K.
      await browser.execute(() => {
        window.dispatchEvent(
          new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true }),
        );
      });
      const paletteInput = await browser.$(".pas-pal-q");
      await paletteInput.waitForDisplayed({ timeout: 5_000 });

      // Buscar la primera sesión por subsecuencia ("cl pk") y presionar Enter.
      await paletteInput.setValue("cl pk");
      await browser.waitUntil(
        async () =>
          browser.execute(
            (id: string) =>
              document
                .querySelector('[data-palette-index="0"]')
                ?.getAttribute("data-palette-id") === `session:${id}`,
            firstSessionId!,
          ),
        {
          timeout: 5_000,
          timeoutMsg: "La primera sesión no quedó primera en la paleta Ctrl+K.",
        },
      );
      await paletteInput.addValue("\uE007");

      await browser.waitUntil(
        async () => {
          const state = await browser.execute(() => {
            const palExists = document.querySelector(".pas-pal") !== null;
            const visiblePanes = Array.from(
              document.querySelectorAll<HTMLElement>(".pas-pane.is-focused"),
            ).filter((el) => el.offsetParent !== null);
            const activeTabText =
              document.querySelector(".pas-tab.is-active")?.textContent ?? "";
            return {
              palExists,
              focusedId: visiblePanes[0]?.getAttribute("data-session-id") ?? null,
              activeTabText,
            };
          });
          return !state.palExists && state.focusedId === firstSessionId;
        },
        {
          timeout: 10_000,
          interval: 100,
          timeoutMsg: "Ctrl+K no enfocó la primera sesión al presionar Enter.",
        },
      );
    } finally {
      await ctx.cleanup();
    }
  });
});
