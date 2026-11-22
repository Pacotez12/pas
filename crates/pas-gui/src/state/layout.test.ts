import { describe, expect, it } from "vitest";
import {
  closePane,
  cycleTab,
  focusSession,
  listTabSessions,
  openSessionTab,
  replacePaneSession,
  resizeSplit,
  restoreLayout,
  splitPane,
  type GuiLayout,
} from "./layout";

describe("state/layout", () => {
  it("split_then_close_collapses", () => {
    let layout: GuiLayout = { tabs: [], activeTabId: null };
    layout = openSessionTab(layout, "s1");
    expect(layout.tabs).toHaveLength(1);
    expect(layout.tabs[0].root).toEqual({ kind: "pane", sessionId: "s1" });

    layout = splitPane(layout, "s1", "row", "s2");
    expect(layout.tabs[0].root).toEqual({
      kind: "split",
      dir: "row",
      children: [
        { kind: "pane", sessionId: "s1" },
        { kind: "pane", sessionId: "s2" },
      ],
      sizes: [50, 50],
    });
    expect(layout.tabs[0].focusedSessionId).toBe("s2");

    layout = closePane(layout, "s2");
    expect(layout.tabs).toHaveLength(1);
    expect(layout.tabs[0].root).toEqual({ kind: "pane", sessionId: "s1" });
    expect(layout.tabs[0].focusedSessionId).toBe("s1");

    layout = closePane(layout, "s1");
    expect(layout.tabs).toHaveLength(0);
    expect(layout.activeTabId).toBeNull();
  });

  it("restore_drops_missing_sessions", () => {
    const saved: GuiLayout = {
      activeTabId: "t1",
      tabs: [
        {
          id: "t1",
          focusedSessionId: "s2",
          root: {
            kind: "split",
            dir: "row",
            sizes: [60, 40],
            children: [
              { kind: "pane", sessionId: "s1" },
              {
                kind: "split",
                dir: "col",
                sizes: [50, 50],
                children: [
                  { kind: "pane", sessionId: "s2" },
                  { kind: "pane", sessionId: "s3" },
                ],
              },
            ],
          },
        },
        {
          id: "t2",
          focusedSessionId: "s4",
          root: { kind: "pane", sessionId: "s4" },
        },
      ],
    };

    const restored = restoreLayout(saved, ["s1", "s3"]);
    expect(restored.tabs).toHaveLength(1);
    expect(restored.activeTabId).toBe("t1");
    expect(restored.tabs[0].root).toEqual({
      kind: "split",
      dir: "row",
      sizes: [60, 40],
      children: [
        { kind: "pane", sessionId: "s1" },
        { kind: "pane", sessionId: "s3" },
      ],
    });
    expect(restored.tabs[0].focusedSessionId).toBe("s1");
  });

  it("restore_handles_corrupt_or_empty_input", () => {
    expect(restoreLayout(null, ["s1"])).toEqual({ tabs: [], activeTabId: null });
    expect(restoreLayout("bad", ["s1"])).toEqual({ tabs: [], activeTabId: null });
    expect(restoreLayout({ tabs: "nope" }, ["s1"])).toEqual({ tabs: [], activeTabId: null });
  });

  it("openSessionTab focuses existing tab instead of duplicating", () => {
    let layout: GuiLayout = { tabs: [], activeTabId: null };
    layout = openSessionTab(layout, "s1");
    layout = openSessionTab(layout, "s2");
    expect(layout.tabs).toHaveLength(2);
    const firstTabId = layout.tabs[0].id;

    layout = openSessionTab(layout, "s1");
    expect(layout.tabs).toHaveLength(2);
    expect(layout.activeTabId).toBe(firstTabId);
    expect(layout.tabs[0].focusedSessionId).toBe("s1");
  });

  it("cycleTab wraps around forward and backward", () => {
    let layout: GuiLayout = { tabs: [], activeTabId: null };
    layout = openSessionTab(layout, "s1");
    layout = openSessionTab(layout, "s2");
    layout = openSessionTab(layout, "s3");
    const [t1, t2, t3] = layout.tabs.map((t) => t.id);

    expect(layout.activeTabId).toBe(t3);
    layout = cycleTab(layout, 1);
    expect(layout.activeTabId).toBe(t1);
    layout = cycleTab(layout, -1);
    expect(layout.activeTabId).toBe(t3);
    layout = focusSession(layout, "s2");
    expect(layout.activeTabId).toBe(t2);
  });

  it("resizeSplit updates sizes at given path and listTabSessions collects ids", () => {
    let layout: GuiLayout = { tabs: [], activeTabId: null };
    layout = openSessionTab(layout, "s1");
    layout = splitPane(layout, "s1", "row", "s2");
    const tabId = layout.tabs[0].id;

    layout = resizeSplit(layout, tabId, [], [35, 65]);
    expect(layout.tabs[0].root).toEqual({
      kind: "split",
      dir: "row",
      children: [
        { kind: "pane", sessionId: "s1" },
        { kind: "pane", sessionId: "s2" },
      ],
      sizes: [35, 65],
    });
    expect(listTabSessions(layout.tabs[0].root)).toEqual(["s1", "s2"]);
  });

  it("replacePaneSession swaps session id in place preserving splits and focus", () => {
    let layout: GuiLayout = { tabs: [], activeTabId: null };
    layout = openSessionTab(layout, "s1");
    layout = splitPane(layout, "s1", "row", "s2");
    layout = resizeSplit(layout, layout.tabs[0].id, [], [35, 65]);

    const replaced = replacePaneSession(layout, "s2", "s3");
    expect(replaced.tabs[0].root).toEqual({
      kind: "split",
      dir: "row",
      children: [
        { kind: "pane", sessionId: "s1" },
        { kind: "pane", sessionId: "s3" },
      ],
      sizes: [35, 65],
    });
    expect(replaced.tabs[0].focusedSessionId).toBe("s3");
  });
});
