export type AgentPreset = "claude" | "codex" | "agy" | "custom";

export const PRESET_SHORT: Record<Exclude<AgentPreset, "custom">, string> = {
  claude: "claude",
  codex: "codex",
  agy: "agy",
};

export function splitCommandString(raw: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  for (let i = 0; i < raw.length; i += 1) {
    const ch = raw[i];
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else {
        current += ch;
      }
    } else if (ch === "'" || ch === '"') {
      quote = ch;
    } else if (/\s/.test(ch)) {
      if (current) {
        parts.push(current);
        current = "";
      }
    } else {
      current += ch;
    }
  }
  if (current) parts.push(current);
  return parts;
}

export function buildNewSessionCommand(
  agent: AgentPreset,
  customCmd: string,
  conversationId: string | null,
): string[] {
  if (agent === "custom") {
    return splitCommandString(customCmd.trim());
  }
  if (agent === "claude") {
    const trimmed = conversationId?.trim();
    return trimmed ? ["claude", "--resume", trimmed] : ["claude"];
  }
  return [PRESET_SHORT[agent]];
}

function isClaudeBinary(program: string): boolean {
  const parts = program.split("/");
  return parts[parts.length - 1] === "claude";
}

function hasResumeOrContinueFlag(args: string[]): boolean {
  return args.some(
    (arg) =>
      arg === "--resume" ||
      arg.startsWith("--resume=") ||
      arg === "-r" ||
      arg === "--continue" ||
      arg.startsWith("--continue=") ||
      arg === "-c",
  );
}

export function buildReopenCommand(command: string[]): string[] {
  if (command.length === 0) return [];
  if (isClaudeBinary(command[0]) && !hasResumeOrContinueFlag(command.slice(1))) {
    return [...command, "--continue"];
  }
  return [...command];
}
