import { describe, expect, it } from "vitest";
import { buildNewSessionCommand, buildReopenCommand } from "./sessionCommand";

describe("state/sessionCommand", () => {
  describe("buildReopenCommand", () => {
    it("agrega --continue cuando el comando es claude sin --resume ni --continue", () => {
      expect(buildReopenCommand(["claude"])).toEqual(["claude", "--continue"]);
      expect(buildReopenCommand(["/usr/local/bin/claude", "--model", "opus"])).toEqual([
        "/usr/local/bin/claude",
        "--model",
        "opus",
        "--continue",
      ]);
    });

    it("no duplica --continue si claude ya trae --continue o --resume", () => {
      expect(buildReopenCommand(["claude", "--continue"])).toEqual(["claude", "--continue"]);
      expect(buildReopenCommand(["claude", "--resume", "abc-123"])).toEqual([
        "claude",
        "--resume",
        "abc-123",
      ]);
      expect(buildReopenCommand(["claude", "--resume=abc-123"])).toEqual([
        "claude",
        "--resume=abc-123",
      ]);
    });

    it("deja intactos los comandos que no son claude", () => {
      expect(buildReopenCommand(["codex"])).toEqual(["codex"]);
      expect(buildReopenCommand(["agy"])).toEqual(["agy"]);
      expect(buildReopenCommand(["bash", "-l"])).toEqual(["bash", "-l"]);
      expect(buildReopenCommand([])).toEqual([]);
    });
  });

  describe("buildNewSessionCommand", () => {
    it("arma claude con --resume cuando se elige una conversación previa", () => {
      expect(buildNewSessionCommand("claude", "", "conv-123")).toEqual([
        "claude",
        "--resume",
        "conv-123",
      ]);
    });

    it("arma claude sin flags cuando es una conversación nueva", () => {
      expect(buildNewSessionCommand("claude", "", null)).toEqual(["claude"]);
    });

    it("ignora conversationId para otros agentes y comando personalizado", () => {
      expect(buildNewSessionCommand("codex", "", "conv-123")).toEqual(["codex"]);
      expect(buildNewSessionCommand("agy", "", "conv-123")).toEqual(["agy"]);
      expect(buildNewSessionCommand("custom", "bash -l", "conv-123")).toEqual(["bash", "-l"]);
    });
  });
});
