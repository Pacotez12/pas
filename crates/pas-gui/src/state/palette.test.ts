import { describe, expect, it } from "vitest";
import {
  buildPaletteItems,
  filterPaletteItems,
  matchWordSubsequences,
  type PaletteAction,
} from "./palette";
import type { ProjectInfo, SessionInfo } from "../api/pas";

const sampleSessions: SessionInfo[] = [
  {
    id: "s-1",
    title: "agy · pockt",
    cwd: "/home/u/Proyectos/pockt",
    command: ["agy"],
    state: { state: "running" },
    created_at: 1,
    project_id: "p-1",
    account_id: null,
    mode: "terminal",
    worktree: null,
  },
  {
    id: "s-2",
    title: "claude · pockt",
    cwd: "/home/u/Proyectos/pockt",
    command: ["claude"],
    state: { state: "idle" },
    created_at: 2,
    project_id: "p-1",
    account_id: null,
    mode: "chat",
    worktree: null,
  },
  {
    id: "s-3",
    title: "codex · pas",
    cwd: "/home/u/orca/projects/pas",
    command: ["codex"],
    state: { state: "waiting" },
    created_at: 3,
    project_id: "p-2",
    account_id: null,
    mode: "terminal",
    worktree: null,
  },
];

const sampleProjects: ProjectInfo[] = [
  {
    id: "p-1",
    name: "pockt",
    path: "/home/u/Proyectos/pockt",
    group_id: "g-1",
    group_pinned: false,
  },
  {
    id: "p-2",
    name: "pas",
    path: "/home/u/orca/projects/pas",
    group_id: "g-2",
    group_pinned: false,
  },
];

const sampleActions: PaletteAction[] = [
  { id: "new-session", label: "Nueva sesión", shortcut: "Ctrl+Shift+T" },
  { id: "add-project", label: "Agregar proyecto…" },
  { id: "accounts", label: "Cuentas" },
];

describe("matchWordSubsequences", () => {
  it("encuentra subsecuencias de palabras en cualquier orden ('pk ag' -> 'agy · pockt')", () => {
    expect(matchWordSubsequences("pk ag", "agy · pockt")).not.toBeNull();
    expect(matchWordSubsequences("pk ag", "claude · pockt")).toBeNull();
  });

  it("es insensible a mayúsculas y tildes", () => {
    expect(matchWordSubsequences("sesion", "Nueva sesión")).not.toBeNull();
    expect(matchWordSubsequences("POCK", "agy · pockt")).not.toBeNull();
  });

  it("prioriza coincidencias de subcadena contigua sobre subsecuencias dispersas", () => {
    const contiguous = matchWordSubsequences("pas", "pas")!;
    const scattered = matchWordSubsequences("pas", "proyectos abiertos siempre")!;
    expect(contiguous).toBeGreaterThan(scattered);
  });

  it("con query vacía devuelve score neutro", () => {
    expect(matchWordSubsequences("   ", "cualquier cosa")).toBe(0);
  });
});

describe("filterPaletteItems", () => {
  it("agrupa resultados en Sesiones, Proyectos y Acciones y filtra por subsecuencias", () => {
    const items = buildPaletteItems({
      sessions: sampleSessions,
      projects: sampleProjects,
      actions: sampleActions,
    });

    const filtered = filterPaletteItems(items, "pk ag");
    expect(filtered.map((i) => i.id)).toEqual(["session:s-1"]);
  });

  it("con 'pock' encuentra sesiones y proyecto pockt manteniendo el orden de secciones", () => {
    const items = buildPaletteItems({
      sessions: sampleSessions,
      projects: sampleProjects,
      actions: sampleActions,
    });

    const filtered = filterPaletteItems(items, "pock");
    expect(filtered.map((i) => i.section)).toEqual([
      "sessions",
      "sessions",
      "projects",
    ]);
    expect(filtered.map((i) => i.id)).toContain("session:s-1");
    expect(filtered.map((i) => i.id)).toContain("session:s-2");
    expect(filtered.map((i) => i.id)).toContain("project:p-1");
  });

  it("con query vacía devuelve todas las secciones en orden", () => {
    const items = buildPaletteItems({
      sessions: sampleSessions,
      projects: sampleProjects,
      actions: sampleActions,
    });

    const filtered = filterPaletteItems(items, "");
    expect(filtered).toHaveLength(sampleSessions.length + sampleProjects.length + sampleActions.length);
    expect(filtered[0].section).toBe("sessions");
    expect(filtered[filtered.length - 1].section).toBe("actions");
  });
});
