import type { ProjectInfo, SessionInfo } from "../api/pas";

export type PaletteSection = "sessions" | "projects" | "actions";

export interface PaletteAction {
  id: string;
  label: string;
  shortcut?: string;
}

export type PaletteItem =
  | {
      id: string;
      section: "sessions";
      label: string;
      searchText: string;
      session: SessionInfo;
    }
  | {
      id: string;
      section: "projects";
      label: string;
      searchText: string;
      project: ProjectInfo;
    }
  | {
      id: string;
      section: "actions";
      label: string;
      searchText: string;
      action: PaletteAction;
    };

function normalize(input: string): string {
  return input
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "");
}

function scoreWordSubsequence(word: string, text: string): number | null {
  if (!word) return 0;

  const exactIdx = text.indexOf(word);
  if (exactIdx !== -1) {
    const atBoundary =
      exactIdx === 0 || /[\s·/._-]/.test(text[exactIdx - 1] ?? "");
    return (atBoundary ? 200 : 120) - exactIdx * 0.5;
  }

  let ti = 0;
  let score = 0;
  let prevMatch = -2;
  let firstMatch = -1;

  for (let wi = 0; wi < word.length; wi += 1) {
    const ch = word[wi];
    const found = text.indexOf(ch, ti);
    if (found === -1) return null;
    if (firstMatch === -1) firstMatch = found;

    if (found === prevMatch + 1) {
      score += 12;
    } else if (found === 0 || /[\s·/._-]/.test(text[found - 1] ?? "")) {
      score += 10;
    } else {
      score += 2;
    }
    prevMatch = found;
    ti = found + 1;
  }

  const span = prevMatch - firstMatch + 1;
  return score - (span - word.length) * 0.5;
}

export function matchWordSubsequences(query: string, candidate: string): number | null {
  const words = normalize(query).trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return 0;

  const normCandidate = normalize(candidate);
  let total = 0;
  for (const word of words) {
    const s = scoreWordSubsequence(word, normCandidate);
    if (s === null) return null;
    total += s;
  }
  return total;
}

export function buildPaletteItems(params: {
  sessions: SessionInfo[];
  projects: ProjectInfo[];
  actions: PaletteAction[];
}): PaletteItem[] {
  const items: PaletteItem[] = [];

  for (const session of params.sessions) {
    items.push({
      id: `session:${session.id}`,
      section: "sessions",
      label: session.title,
      searchText: `${session.title} ${session.worktree?.branch ?? ""}`,
      session,
    });
  }

  for (const project of params.projects) {
    items.push({
      id: `project:${project.id}`,
      section: "projects",
      label: project.name,
      searchText: `${project.name} ${project.path}`,
      project,
    });
  }

  for (const action of params.actions) {
    items.push({
      id: `action:${action.id}`,
      section: "actions",
      label: action.label,
      searchText: `${action.label} ${action.shortcut ?? ""}`,
      action,
    });
  }

  return items;
}

const SECTION_ORDER: Record<PaletteSection, number> = {
  sessions: 0,
  projects: 1,
  actions: 2,
};

export function filterPaletteItems(items: PaletteItem[], query: string): PaletteItem[] {
  const trimmed = query.trim();
  if (!trimmed) return items;

  const scored: Array<{ item: PaletteItem; score: number; index: number }> = [];
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    const score = matchWordSubsequences(trimmed, item.searchText);
    if (score !== null) {
      scored.push({ item, score, index: i });
    }
  }

  scored.sort((a, b) => {
    const secDiff = SECTION_ORDER[a.item.section] - SECTION_ORDER[b.item.section];
    if (secDiff !== 0) return secDiff;
    if (b.score !== a.score) return b.score - a.score;
    return a.index - b.index;
  });

  return scored.map((entry) => entry.item);
}
