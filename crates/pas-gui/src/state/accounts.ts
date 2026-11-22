import type { AccountInfo, AgentKind, Quota } from "../api/pas";
import type { AgentPreset } from "./sessionCommand";

export type QuotaLevel = "normal" | "warn" | "high";

export interface FormattedQuotaWindow {
  key: "5h" | "7d";
  text: string;
  level: QuotaLevel;
}

export interface FormattedQuota {
  hasData: boolean;
  windows: FormattedQuotaWindow[];
}

export function quotaLevel(percent: number): QuotaLevel {
  if (percent >= 80) return "high";
  if (percent >= 50) return "warn";
  return "normal";
}

export function formatQuotaWindows(quota: Quota | null | undefined): FormattedQuota {
  if (!quota) {
    return { hasData: false, windows: [] };
  }
  const windows: FormattedQuotaWindow[] = [];
  if (quota.five_h) {
    const pct = Math.round(quota.five_h.used_percent);
    windows.push({
      key: "5h",
      text: `5h ${pct}%`,
      level: quotaLevel(quota.five_h.used_percent),
    });
  }
  if (quota.week) {
    const pct = Math.round(quota.week.used_percent);
    windows.push({
      key: "7d",
      text: `7d ${pct}%`,
      level: quotaLevel(quota.week.used_percent),
    });
  }
  return {
    hasData: windows.length > 0,
    windows,
  };
}

export function formatQuotaInline(
  quota: Quota | null | undefined,
  noDataLabel: string,
): string {
  const formatted = formatQuotaWindows(quota);
  if (!formatted.hasData) return noDataLabel;
  return formatted.windows.map((w) => w.text).join(" · ");
}

const AGENT_SHORT_LABEL: Record<AgentKind, string> = {
  claude: "Claude",
  codex: "Codex",
  agy: "agy",
};

export function quotaPillLabel(account: AccountInfo): string {
  if (
    account.agent === "agy" &&
    account.name.trim().toLowerCase() === "predeterminada"
  ) {
    return "agy";
  }
  return `${AGENT_SHORT_LABEL[account.agent]} · ${account.name}`;
}

export function filterAccountsByAgent(
  accounts: AccountInfo[],
  agent: AgentPreset,
): AccountInfo[] {
  if (agent === "custom") return [];
  return accounts.filter((a) => a.agent === agent);
}

export interface HooksTargetInfo {
  filePath: string;
  backupName: string;
  wrapsStatusline: boolean;
}

function formatUtcYyyymmdd(date: Date): string {
  const y = String(date.getUTCFullYear()).padStart(4, "0");
  const m = String(date.getUTCMonth() + 1).padStart(2, "0");
  const d = String(date.getUTCDate()).padStart(2, "0");
  return `${y}${m}${d}`;
}

export function hooksTargetInfo(
  account: AccountInfo,
  now: Date = new Date(),
): HooksTargetInfo {
  const base = account.config_dir.replace(/\/+$/, "");
  const yyyymmdd = formatUtcYyyymmdd(now);

  if (account.agent === "claude") {
    return {
      filePath: `${base}/settings.json`,
      backupName: `settings.json.bak-pas-${yyyymmdd}`,
      wrapsStatusline: true,
    };
  }
  if (account.agent === "codex") {
    return {
      filePath: `${base}/hooks.json`,
      backupName: `hooks.json.bak-pas-${yyyymmdd}`,
      wrapsStatusline: false,
    };
  }
  const isGeminiRoot = base.endsWith("/.gemini") || base === ".gemini";
  return {
    filePath: isGeminiRoot ? `${base}/config/hooks.json` : `${base}/hooks.json`,
    backupName: `hooks.json.bak-pas-${yyyymmdd}`,
    wrapsStatusline: true,
  };
}
