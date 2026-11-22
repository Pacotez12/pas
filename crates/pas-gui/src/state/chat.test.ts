import { describe, expect, it } from "vitest";
import type { ChatEvent } from "../api/pas";
import {
  buildVisualSrcDoc,
  createInitialChatState,
  hydrateChatHistory,
  reduceChatEvent,
} from "./chat";

describe("state/chat", () => {
  it("une chunks de agente consecutivos en un solo mensaje hasta agent_message_end", () => {
    let state = createInitialChatState();
    state = reduceChatEvent(state, { type: "user_message", text: "Hola" });
    state = reduceChatEvent(state, { type: "agent_chunk", text: "Busco " });
    state = reduceChatEvent(state, { type: "agent_chunk", text: "el archivo." });

    expect(state.items).toHaveLength(2);
    expect(state.items[0]).toMatchObject({ kind: "user", text: "Hola" });
    expect(state.items[1]).toMatchObject({
      kind: "agent",
      text: "Busco el archivo.",
      streaming: true,
    });

    state = reduceChatEvent(state, { type: "agent_message_end" });
    expect(state.items[1]).toMatchObject({
      kind: "agent",
      text: "Busco el archivo.",
      streaming: false,
    });

    // Un chunk posterior después de agent_message_end abre una burbuja nueva.
    state = reduceChatEvent(state, { type: "agent_chunk", text: "Listo." });
    expect(state.items).toHaveLength(3);
    expect(state.items[2]).toMatchObject({
      kind: "agent",
      text: "Listo.",
      streaming: true,
    });
  });

  it("actualiza una llamada a herramienta por id sin duplicar filas", () => {
    let state = createInitialChatState();
    state = reduceChatEvent(state, {
      type: "tool_call",
      id: "tc-1",
      title: "Leyó lib/main.dart",
      status: "in_progress",
    });
    state = reduceChatEvent(state, {
      type: "tool_call",
      id: "tc-1",
      title: "",
      status: "completed",
    });

    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({
      kind: "tool",
      id: "tc-1",
      title: "Leyó lib/main.dart",
      status: "completed",
    });
  });

  it("cambia la tarjeta de permiso de pendiente a resuelta cuando llega permission_resolved", () => {
    let state = createInitialChatState();
    state = reduceChatEvent(state, {
      type: "permission_request",
      request_id: "perm-1",
      title: "Ejecutar comando",
      detail: "flutter test",
    });

    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({
      kind: "permission",
      requestId: "perm-1",
      title: "Ejecutar comando",
      detail: "flutter test",
      resolution: null,
    });

    state = reduceChatEvent(state, {
      type: "permission_resolved",
      request_id: "perm-1",
      choice: "allow_session",
    });

    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({
      kind: "permission",
      requestId: "perm-1",
      resolution: "allow_session",
    });
  });

  it("calcula líneas agregadas y borradas en un diff unificado", () => {
    let state = createInitialChatState();
    state = reduceChatEvent(state, {
      type: "diff",
      path: "src/app.ts",
      unified: "--- a/src/app.ts\n+++ b/src/app.ts\n-const a = 1;\n+const a = 2;\n+const b = 3;\n",
    });

    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({
      kind: "diff",
      path: "src/app.ts",
      additions: 2,
      deletions: 1,
    });
  });

  it("hidrata historial + eventos en vivo solapados sin duplicar ni perder eventos", () => {
    const history: ChatEvent[] = [
      { type: "user_message", text: "Probá" },
      { type: "agent_chunk", text: "Parte 1 " },
      { type: "agent_chunk", text: "Parte 2" },
    ];
    // El evento "Parte 2" llegó también por stream mientras se cargaba chat.history,
    // seguido de un chunk nuevo y el cierre del turno.
    const buffered: ChatEvent[] = [
      { type: "agent_chunk", text: "Parte 2" },
      { type: "agent_chunk", text: " Parte 3" },
      { type: "agent_message_end" },
    ];

    const state = hydrateChatHistory(history, buffered);
    expect(state.seq).toBe(5);
    expect(state.items).toHaveLength(2);
    expect(state.items[0]).toMatchObject({ kind: "user", text: "Probá" });
    expect(state.items[1]).toMatchObject({
      kind: "agent",
      text: "Parte 1 Parte 2 Parte 3",
      streaming: false,
    });
  });

  it("ignora eventos repetidos si se rehidrata con el mismo historial", () => {
    const history: ChatEvent[] = [
      { type: "user_message", text: "Hola" },
      { type: "visual", file: "mockup.html" },
      { type: "agent_message_end" },
    ];

    const first = hydrateChatHistory(history, []);
    const second = hydrateChatHistory(history, history);

    expect(second.items).toEqual(first.items);
    expect(second.seq).toBe(3);
  });

  it("inyecta una CSP sin red en el documento HTML del visual", () => {
    const wrapped = buildVisualSrcDoc("<html><head><title>x</title></head><body>Hola</body></html>");
    expect(wrapped).toContain('http-equiv="Content-Security-Policy"');
    expect(wrapped).toContain("default-src 'none'");
    expect(wrapped).toContain("<body>Hola</body>");

    const fragment = buildVisualSrcDoc("<div>Sin head</div>");
    expect(fragment).toContain("default-src 'none'");
    expect(fragment).toContain("<div>Sin head</div>");
  });
});
