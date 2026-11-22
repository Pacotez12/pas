export type SplitDir = "row" | "col";

export type LayoutNode =
  | { kind: "pane"; sessionId: string }
  | { kind: "split"; dir: SplitDir; children: LayoutNode[]; sizes: number[] };

export interface TabLayout {
  id: string;
  root: LayoutNode;
  focusedSessionId: string;
}

export interface GuiLayout {
  tabs: TabLayout[];
  activeTabId: string | null;
}

let tabCounter = 0;

function nextTabId(existing: TabLayout[]): string {
  const used = new Set(existing.map((t) => t.id));
  while (true) {
    tabCounter += 1;
    const candidate = `tab-${tabCounter}`;
    if (!used.has(candidate)) {
      return candidate;
    }
  }
}

export function listTabSessions(node: LayoutNode): string[] {
  if (node.kind === "pane") {
    return [node.sessionId];
  }
  return node.children.flatMap(listTabSessions);
}

export function findTabBySession(
  layout: GuiLayout,
  sessionId: string,
): TabLayout | undefined {
  return layout.tabs.find((tab) =>
    listTabSessions(tab.root).includes(sessionId),
  );
}

export function openSessionTab(layout: GuiLayout, sessionId: string): GuiLayout {
  const existing = findTabBySession(layout, sessionId);
  if (existing) {
    return {
      activeTabId: existing.id,
      tabs: layout.tabs.map((tab) =>
        tab.id === existing.id ? { ...tab, focusedSessionId: sessionId } : tab,
      ),
    };
  }
  const id = nextTabId(layout.tabs);
  const newTab: TabLayout = {
    id,
    root: { kind: "pane", sessionId },
    focusedSessionId: sessionId,
  };
  return {
    tabs: [...layout.tabs, newTab],
    activeTabId: id,
  };
}

function splitNode(
  node: LayoutNode,
  targetSessionId: string,
  dir: SplitDir,
  newSessionId: string,
): { node: LayoutNode; inserted: boolean } {
  if (node.kind === "pane") {
    if (node.sessionId === targetSessionId) {
      return {
        node: {
          kind: "split",
          dir,
          children: [node, { kind: "pane", sessionId: newSessionId }],
          sizes: [50, 50],
        },
        inserted: true,
      };
    }
    return { node, inserted: false };
  }

  let inserted = false;
  const children = node.children.map((child) => {
    if (inserted) return child;
    const res = splitNode(child, targetSessionId, dir, newSessionId);
    if (res.inserted) inserted = true;
    return res.node;
  });

  return {
    node: inserted ? { ...node, children } : node,
    inserted,
  };
}

export function splitPane(
  layout: GuiLayout,
  targetSessionId: string,
  dir: SplitDir,
  newSessionId: string,
): GuiLayout {
  let didInsert = false;
  let targetTabId = layout.activeTabId;

  const tabs = layout.tabs.map((tab) => {
    if (didInsert) return tab;
    const res = splitNode(tab.root, targetSessionId, dir, newSessionId);
    if (res.inserted) {
      didInsert = true;
      targetTabId = tab.id;
      return {
        ...tab,
        root: res.node,
        focusedSessionId: newSessionId,
      };
    }
    return tab;
  });

  if (!didInsert) {
    return openSessionTab(layout, newSessionId);
  }

  return {
    tabs,
    activeTabId: targetTabId,
  };
}

function normalizeSizes(sizes: number[], count: number): number[] {
  if (count === 0) return [];
  if (sizes.length !== count) {
    const equal = 100 / count;
    return Array.from({ length: count }, () => equal);
  }
  const total = sizes.reduce((acc, n) => acc + (n > 0 ? n : 0), 0);
  if (total <= 0) {
    const equal = 100 / count;
    return Array.from({ length: count }, () => equal);
  }
  return sizes.map((n) => (n / total) * 100);
}

function pruneNode(
  node: LayoutNode,
  keep: (sessionId: string) => boolean,
): LayoutNode | null {
  if (node.kind === "pane") {
    return keep(node.sessionId) ? node : null;
  }

  const nextChildren: LayoutNode[] = [];
  const nextSizes: number[] = [];

  for (let i = 0; i < node.children.length; i += 1) {
    const pruned = pruneNode(node.children[i], keep);
    if (pruned !== null) {
      nextChildren.push(pruned);
      nextSizes.push(node.sizes[i] ?? 100 / node.children.length);
    }
  }

  if (nextChildren.length === 0) {
    return null;
  }
  if (nextChildren.length === 1) {
    return nextChildren[0];
  }

  return {
    kind: "split",
    dir: node.dir,
    children: nextChildren,
    sizes: normalizeSizes(nextSizes, nextChildren.length),
  };
}

export function closePane(layout: GuiLayout, sessionId: string): GuiLayout {
  const nextTabs: TabLayout[] = [];
  let removedActiveIndex = -1;

  layout.tabs.forEach((tab, idx) => {
    const pruned = pruneNode(tab.root, (id) => id !== sessionId);
    if (pruned === null) {
      if (tab.id === layout.activeTabId) {
        removedActiveIndex = idx;
      }
      return;
    }
    const sessions = listTabSessions(pruned);
    const focusedSessionId = sessions.includes(tab.focusedSessionId)
      ? tab.focusedSessionId
      : sessions[0];
    nextTabs.push({
      ...tab,
      root: pruned,
      focusedSessionId,
    });
  });

  if (nextTabs.length === 0) {
    return { tabs: [], activeTabId: null };
  }

  let activeTabId = layout.activeTabId;
  if (!activeTabId || !nextTabs.some((t) => t.id === activeTabId)) {
    const fallbackIdx =
      removedActiveIndex >= 0
        ? Math.min(removedActiveIndex, nextTabs.length - 1)
        : 0;
    activeTabId = nextTabs[fallbackIdx].id;
  }

  return { tabs: nextTabs, activeTabId };
}

export function closeTab(layout: GuiLayout, tabId: string): GuiLayout {
  const idx = layout.tabs.findIndex((t) => t.id === tabId);
  if (idx === -1) return layout;

  const nextTabs = layout.tabs.filter((t) => t.id !== tabId);
  if (nextTabs.length === 0) {
    return { tabs: [], activeTabId: null };
  }

  const activeTabId =
    layout.activeTabId === tabId
      ? nextTabs[Math.min(idx, nextTabs.length - 1)].id
      : layout.activeTabId;

  return { tabs: nextTabs, activeTabId };
}

export function focusSession(layout: GuiLayout, sessionId: string): GuiLayout {
  const tab = findTabBySession(layout, sessionId);
  if (!tab) return layout;
  return {
    activeTabId: tab.id,
    tabs: layout.tabs.map((t) =>
      t.id === tab.id ? { ...t, focusedSessionId: sessionId } : t,
    ),
  };
}

function replaceNodeSession(
  node: LayoutNode,
  oldSessionId: string,
  newSessionId: string,
): LayoutNode {
  if (node.kind === "pane") {
    return node.sessionId === oldSessionId
      ? { kind: "pane", sessionId: newSessionId }
      : node;
  }
  return {
    ...node,
    children: node.children.map((child) =>
      replaceNodeSession(child, oldSessionId, newSessionId),
    ),
  };
}

export function replacePaneSession(
  layout: GuiLayout,
  oldSessionId: string,
  newSessionId: string,
): GuiLayout {
  return {
    ...layout,
    tabs: layout.tabs.map((tab) => ({
      ...tab,
      root: replaceNodeSession(tab.root, oldSessionId, newSessionId),
      focusedSessionId:
        tab.focusedSessionId === oldSessionId
          ? newSessionId
          : tab.focusedSessionId,
    })),
  };
}

export function cycleTab(layout: GuiLayout, delta: 1 | -1): GuiLayout {
  if (layout.tabs.length <= 1) return layout;
  const currentIdx = layout.tabs.findIndex((t) => t.id === layout.activeTabId);
  const baseIdx = currentIdx === -1 ? 0 : currentIdx;
  const nextIdx =
    (baseIdx + delta + layout.tabs.length) % layout.tabs.length;
  return {
    ...layout,
    activeTabId: layout.tabs[nextIdx].id,
  };
}

function updateSplitSizes(
  node: LayoutNode,
  path: number[],
  sizes: number[],
): LayoutNode {
  if (node.kind !== "split") return node;
  if (path.length === 0) {
    return {
      ...node,
      sizes: normalizeSizes(sizes, node.children.length),
    };
  }
  const [head, ...rest] = path;
  if (head < 0 || head >= node.children.length) return node;
  return {
    ...node,
    children: node.children.map((child, idx) =>
      idx === head ? updateSplitSizes(child, rest, sizes) : child,
    ),
  };
}

export function resizeSplit(
  layout: GuiLayout,
  tabId: string,
  path: number[],
  sizes: number[],
): GuiLayout {
  return {
    ...layout,
    tabs: layout.tabs.map((tab) =>
      tab.id === tabId
        ? { ...tab, root: updateSplitSizes(tab.root, path, sizes) }
        : tab,
    ),
  };
}

function parseNode(raw: unknown): LayoutNode | null {
  if (!raw || typeof raw !== "object") return null;
  const obj = raw as Record<string, unknown>;
  if (obj.kind === "pane" && typeof obj.sessionId === "string" && obj.sessionId) {
    return { kind: "pane", sessionId: obj.sessionId };
  }
  if (
    obj.kind === "split" &&
    (obj.dir === "row" || obj.dir === "col") &&
    Array.isArray(obj.children)
  ) {
    const children = obj.children
      .map(parseNode)
      .filter((c): c is LayoutNode => c !== null);
    if (children.length === 0) return null;
    if (children.length === 1) return children[0];
    const rawSizes = Array.isArray(obj.sizes)
      ? obj.sizes.filter((n): n is number => typeof n === "number" && Number.isFinite(n))
      : [];
    return {
      kind: "split",
      dir: obj.dir,
      children,
      sizes: normalizeSizes(rawSizes, children.length),
    };
  }
  return null;
}

export function restoreLayout(
  saved: unknown,
  aliveSessionIds: string[],
): GuiLayout {
  if (!saved || typeof saved !== "object") {
    return { tabs: [], activeTabId: null };
  }
  const obj = saved as Record<string, unknown>;
  if (!Array.isArray(obj.tabs)) {
    return { tabs: [], activeTabId: null };
  }

  const alive = new Set(aliveSessionIds);
  const tabs: TabLayout[] = [];
  const usedTabIds = new Set<string>();

  for (const rawTab of obj.tabs) {
    if (!rawTab || typeof rawTab !== "object") continue;
    const t = rawTab as Record<string, unknown>;
    const parsedRoot = parseNode(t.root);
    if (!parsedRoot) continue;
    const pruned = pruneNode(parsedRoot, (id) => alive.has(id));
    if (!pruned) continue;

    const sessions = listTabSessions(pruned);
    const id =
      typeof t.id === "string" && t.id && !usedTabIds.has(t.id)
        ? t.id
        : nextTabId(tabs);
    usedTabIds.add(id);

    const focusedSessionId =
      typeof t.focusedSessionId === "string" &&
      sessions.includes(t.focusedSessionId)
        ? t.focusedSessionId
        : sessions[0];

    tabs.push({ id, root: pruned, focusedSessionId });
  }

  if (tabs.length === 0) {
    return { tabs: [], activeTabId: null };
  }

  const activeTabId =
    typeof obj.activeTabId === "string" &&
    tabs.some((t) => t.id === obj.activeTabId)
      ? obj.activeTabId
      : tabs[0].id;

  return { tabs, activeTabId };
}
