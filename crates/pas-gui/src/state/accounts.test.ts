import { describe, expect, it } from "vitest";
import type { AccountInfo, Quota } from "../api/pas";
import {
  filterAccountsByAgent,
  formatQuotaInline,
  formatQuotaWindows,
  hooksTargetInfo,
  quotaLevel,
  quotaPillLabel,
} from "./accounts";

function makeAccount(overrides: Partial<AccountInfo> & Pick<AccountInfo, "id" | "agent" | "name">): AccountInfo {
  return {
    config_dir: `/tmp/acc-${overrides.id}`,
    owned: false,
    created_at: 1_700_000_000,
    hooks_installed: false,
    ...overrides,
  };
}

describe("quotaLevel", () => {
  it("clasifica umbrales 50 y 80 correctamente", () => {
    expect(quotaLevel(0)).toBe("normal");
    expect(quotaLevel(49.9)).toBe("normal");
    expect(quotaLevel(50)).toBe("warn");
    expect(quotaLevel(79.9)).toBe("warn");
    expect(quotaLevel(80)).toBe("high");
    expect(quotaLevel(100)).toBe("high");
  });
});

describe("formatQuotaWindows", () => {
  it("devuelve sin datos cuando no hay cuota o ambas ventanas faltan", () => {
    expect(formatQuotaWindows(null)).toEqual({ hasData: false, windows: [] });
    expect(formatQuotaWindows(undefined)).toEqual({ hasData: false, windows: [] });
    expect(
      formatQuotaWindows({ five_h: null, week: null, updated_at: 100 }),
    ).toEqual({ hasData: false, windows: [] });
  });

  it("formatea 5h y 7d con redondeo y nivel de color", () => {
    const quota: Quota = {
      five_h: { used_percent: 32.4, resets_at: 1000 },
      week: { used_percent: 60.6, resets_at: 2000 },
      updated_at: 500,
    };
    expect(formatQuotaWindows(quota)).toEqual({
      hasData: true,
      windows: [
        { key: "5h", text: "5h 32%", level: "normal" },
        { key: "7d", text: "7d 61%", level: "warn" },
      ],
    });
  });

  it("marca rojo desde 80% y soporta una sola ventana disponible", () => {
    const quota: Quota = {
      five_h: { used_percent: 86, resets_at: 1000 },
      week: null,
      updated_at: 500,
    };
    expect(formatQuotaWindows(quota)).toEqual({
      hasData: true,
      windows: [{ key: "5h", text: "5h 86%", level: "high" }],
    });
  });
});

describe("formatQuotaInline", () => {
  it("devuelve texto compacto para selector de cuenta", () => {
    expect(formatQuotaInline(null, "sin datos")).toBe("sin datos");
    expect(
      formatQuotaInline(
        {
          five_h: { used_percent: 32, resets_at: 1 },
          week: { used_percent: 61, resets_at: 2 },
          updated_at: 3,
        },
        "sin datos",
      ),
    ).toBe("5h 32% · 7d 61%");
  });
});

describe("quotaPillLabel", () => {
  it("formatea etiquetas de barra de cuotas según el agente", () => {
    expect(quotaPillLabel(makeAccount({ id: "1", agent: "claude", name: "Personal" }))).toBe(
      "Claude · Personal",
    );
    expect(quotaPillLabel(makeAccount({ id: "2", agent: "codex", name: "Trabajo" }))).toBe(
      "Codex · Trabajo",
    );
    expect(quotaPillLabel(makeAccount({ id: "3", agent: "agy", name: "Predeterminada" }))).toBe(
      "agy",
    );
    expect(quotaPillLabel(makeAccount({ id: "4", agent: "agy", name: "Custom" }))).toBe(
      "agy · Custom",
    );
  });
});

describe("filterAccountsByAgent", () => {
  const accounts: AccountInfo[] = [
    makeAccount({ id: "c1", agent: "claude", name: "Personal" }),
    makeAccount({ id: "c2", agent: "claude", name: "Trabajo" }),
    makeAccount({ id: "x1", agent: "codex", name: "Trabajo" }),
    makeAccount({ id: "a1", agent: "agy", name: "Predeterminada" }),
  ];

  it("filtra cuentas solo para el agente elegido", () => {
    expect(filterAccountsByAgent(accounts, "claude").map((a) => a.id)).toEqual(["c1", "c2"]);
    expect(filterAccountsByAgent(accounts, "codex").map((a) => a.id)).toEqual(["x1"]);
    expect(filterAccountsByAgent(accounts, "agy").map((a) => a.id)).toEqual(["a1"]);
    expect(filterAccountsByAgent(accounts, "custom")).toEqual([]);
  });
});

describe("hooksTargetInfo", () => {
  it("calcula archivo exacto y nombre de backup por agente", () => {
    const date = new Date(Date.UTC(2026, 10, 22));
    expect(
      hooksTargetInfo(makeAccount({ id: "1", agent: "claude", name: "Trabajo", config_dir: "/tmp/claude-prof" }), date),
    ).toEqual({
      filePath: "/tmp/claude-prof/settings.json",
      backupName: "settings.json.bak-pas-20261122",
      wrapsStatusline: true,
    });
    expect(
      hooksTargetInfo(makeAccount({ id: "2", agent: "codex", name: "Trabajo", config_dir: "/tmp/codex-prof/" }), date),
    ).toEqual({
      filePath: "/tmp/codex-prof/hooks.json",
      backupName: "hooks.json.bak-pas-20261122",
      wrapsStatusline: false,
    });
    expect(
      hooksTargetInfo(makeAccount({ id: "3", agent: "agy", name: "Predeterminada", config_dir: "/tmp/.gemini" }), date),
    ).toEqual({
      filePath: "/tmp/.gemini/config/hooks.json",
      backupName: "hooks.json.bak-pas-20261122",
      wrapsStatusline: true,
    });
  });
});
