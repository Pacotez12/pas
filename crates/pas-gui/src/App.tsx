import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, Plus, Search, Settings, X } from "lucide-react";
import pasMarkUrl from "./assets/pas-mark-sin-broche.svg";
import {
  api,
  type ConnectionStatus,
  type GroupInfo,
  type ProjectInfo,
  type SessionInfo,
} from "./api/pas";
import {
  NewSessionDialog,
  type NewSessionFormResult,
} from "./components/NewSessionDialog";
import { AddProjectDialog, TextPromptDialog } from "./components/ProjectDialogs";
import { Sidebar } from "./components/Sidebar";
import { SplitLayout } from "./components/SplitLayout";
import { Tabs } from "./components/Tabs";
import { es } from "./i18n/es";
import {
  closeDialog,
  initialDialogState,
  openDialog,
} from "./state/dialog";
import {
  closePane,
  closeTab,
  cycleTab,
  focusSession,
  openSessionTab,
  resizeSplit,
  restoreLayout,
  splitPane,
  type GuiLayout,
  type SplitDir,
} from "./state/layout";

type TextPromptState =
  | { kind: "rename-project"; project: ProjectInfo }
  | { kind: "rename-group"; group: GroupInfo }
  | { kind: "new-group-and-move"; project: ProjectInfo };

export function App() {
  const [groups, setGroups] = useState<GroupInfo[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [layout, setLayout] = useState<GuiLayout>({ tabs: [], activeTabId: null });
  const [layoutLoaded, setLayoutLoaded] = useState(false);

  const [connection, setConnection] = useState<ConnectionStatus>("connected");
  const [retryCountdown, setRetryCountdown] = useState<number>(2);
  const [errorBanner, setErrorBanner] = useState<string | null>(null);

  const [newSessionDialog, setNewSessionDialog] = useState(initialDialogState);
  const [newSessionSplit, setNewSessionSplit] = useState<{
    enabled: boolean;
    dir: SplitDir;
    targetSessionId: string | null;
  }>({ enabled: false, dir: "row", targetSessionId: null });
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const [textPrompt, setTextPrompt] = useState<TextPromptState | null>(null);
  const [extraFocusTick, setExtraFocusTick] = useState(0);
  const focusTick = newSessionDialog.focusTick + extraFocusTick;

  const sessionsById = useMemo(
    () => new Map(sessions.map((s) => [s.id, s])),
    [sessions],
  );

  const activeTab = useMemo(
    () => layout.tabs.find((t) => t.id === layout.activeTabId) ?? null,
    [layout],
  );
  const activeSessionId = activeTab?.focusedSessionId ?? null;
  const activeSession = activeSessionId
    ? sessionsById.get(activeSessionId) ?? null
    : null;

  const refreshState = useCallback(async (restoreSaved: boolean) => {
    try {
      const [nextGroups, nextProjects, nextSessions] = await Promise.all([
        api.listGroups(),
        api.listProjects(),
        api.listSessions(),
      ]);
      setGroups(nextGroups);
      setProjects(nextProjects);
      setSessions(nextSessions);
      setConnection("connected");

      const aliveIds = nextSessions.map((s) => s.id);
      if (restoreSaved) {
        try {
          const raw = await api.loadLayout();
          const parsed = raw ? JSON.parse(raw) : null;
          setLayout(restoreLayout(parsed, aliveIds));
        } catch {
          setLayout({ tabs: [], activeTabId: null });
        } finally {
          setLayoutLoaded(true);
        }
      } else {
        setLayout((prev) => restoreLayout(prev, aliveIds));
      }
    } catch (err) {
      setErrorBanner(es.errors.loadFailed(String(err)));
      if (restoreSaved) {
        setLayoutLoaded(true);
      }
    }
  }, []);

  useEffect(() => {
    void refreshState(true);

    let unlisteners: Array<() => void> = [];
    let cancelled = false;

    void (async () => {
      const u1 = await api.onConnection((status) => {
        setConnection(status);
        if (status === "disconnected") {
          setRetryCountdown(2);
        } else {
          setErrorBanner(null);
          void refreshState(false);
        }
      });
      const u2 = await api.onState((ev) => {
        setSessions((prev) =>
          prev.map((s) => (s.id === ev.id ? { ...s, state: ev.state } : s)),
        );
      });
      const u3 = await api.onCreated((ev) => {
        setSessions((prev) =>
          prev.some((s) => s.id === ev.info.id) ? prev : [...prev, ev.info],
        );
      });
      const u4 = await api.onRemoved((ev) => {
        setSessions((prev) => prev.filter((s) => s.id !== ev.id));
        setLayout((prev) => closePane(prev, ev.id));
      });

      if (cancelled) {
        u1();
        u2();
        u3();
        u4();
      } else {
        unlisteners = [u1, u2, u3, u4];
      }
    })();

    return () => {
      cancelled = true;
      for (const u of unlisteners) u();
    };
  }, [refreshState]);

  useEffect(() => {
    if (connection !== "disconnected") return;
    const timer = window.setInterval(() => {
      setRetryCountdown((prev) => (prev <= 1 ? 0 : prev - 1));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [connection]);

  const saveTimerRef = useRef<number | null>(null);
  useEffect(() => {
    if (!layoutLoaded) return;
    if (saveTimerRef.current !== null) {
      window.clearTimeout(saveTimerRef.current);
    }
    saveTimerRef.current = window.setTimeout(() => {
      void api.saveLayout(JSON.stringify(layout)).catch(() => {});
    }, 150);
    return () => {
      if (saveTimerRef.current !== null) {
        window.clearTimeout(saveTimerRef.current);
      }
    };
  }, [layout, layoutLoaded]);

  const openNewSessionModal = useCallback(
    (splitOpts?: { dir: SplitDir; targetSessionId: string | null }) => {
      setNewSessionSplit({
        enabled: Boolean(splitOpts && splitOpts.targetSessionId),
        dir: splitOpts?.dir ?? "row",
        targetSessionId: splitOpts?.targetSessionId ?? activeSessionId,
      });
      setNewSessionDialog((prev) => openDialog(prev));
    },
    [activeSessionId],
  );

  const closeNewSessionModal = useCallback(() => {
    setNewSessionDialog((prev) => closeDialog(prev));
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey) {
        const key = e.key.toLowerCase();
        if (key === "t") {
          e.preventDefault();
          openNewSessionModal();
        } else if (key === "d") {
          e.preventDefault();
          openNewSessionModal({ dir: "row", targetSessionId: activeSessionId });
        } else if (key === "e") {
          e.preventDefault();
          openNewSessionModal({ dir: "col", targetSessionId: activeSessionId });
        } else if (key === "w" && activeSessionId) {
          e.preventDefault();
          setLayout((prev) => closePane(prev, activeSessionId));
        }
      } else if (e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey) {
        if (e.key === "PageUp") {
          e.preventDefault();
          setLayout((prev) => cycleTab(prev, -1));
        } else if (e.key === "PageDown") {
          e.preventDefault();
          setLayout((prev) => cycleTab(prev, 1));
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeSessionId, openNewSessionModal]);

  async function handleCreateSession(form: NewSessionFormResult) {
    const created = await api.createSession({
      projectId: form.projectId,
      command: form.command,
      title: form.title,
      cwd: form.cwd,
    });
    const refreshed = await api.listSessions();
    setSessions(refreshed);

    const targetId = newSessionSplit.targetSessionId ?? activeSessionId;
    if (form.openInSplit && targetId) {
      setLayout((prev) => splitPane(prev, targetId, form.splitDir, created.id));
    } else {
      setLayout((prev) => openSessionTab(prev, created.id));
    }
  }

  async function refreshProjectsAndGroups() {
    const [nextGroups, nextProjects] = await Promise.all([
      api.listGroups(),
      api.listProjects(),
    ]);
    setGroups(nextGroups);
    setProjects(nextProjects);
  }

  async function handleAddProject(path: string, name?: string) {
    await api.addProject(path, name);
    await refreshProjectsAndGroups();
  }

  async function handleMoveProject(projectId: string, groupId: string) {
    try {
      await api.moveProject(projectId, groupId);
      await refreshProjectsAndGroups();
    } catch (err) {
      setErrorBanner(
        es.errors.actionFailed(es.errors.actions.moveProject, String(err)),
      );
    }
  }

  async function handleUnpinProject(projectId: string) {
    try {
      await api.unpinProject(projectId);
      await refreshProjectsAndGroups();
    } catch (err) {
      setErrorBanner(
        es.errors.actionFailed(es.errors.actions.unpinProject, String(err)),
      );
    }
  }

  async function handleRemoveProject(project: ProjectInfo) {
    try {
      await api.removeProject(project.id);
      await refreshProjectsAndGroups();
    } catch (err) {
      setErrorBanner(
        es.errors.actionFailed(es.errors.actions.removeProject, String(err)),
      );
    }
  }

  async function handleTextPromptSubmit(value: string) {
    if (!textPrompt) return;
    if (textPrompt.kind === "rename-project") {
      await api.renameProject(textPrompt.project.id, value);
    } else if (textPrompt.kind === "rename-group") {
      await api.renameGroup(textPrompt.group.id, value);
    } else if (textPrompt.kind === "new-group-and-move") {
      const created = await api.createGroup(value);
      await api.moveProject(textPrompt.project.id, created.id);
    }
    await refreshProjectsAndGroups();
  }

  const isDisconnected = connection === "disconnected";

  return (
    <div className="pas-shell">
      <header className="pas-titlebar">
        <img src={pasMarkUrl} alt="" className="pas-titlebar-logo" />
        <span className="pas-titlebar-name">{es.app.name}</span>
        <span className="pas-spacer" />
        <button
          type="button"
          className="pas-icon-btn"
          title={es.app.searchTooltip}
          aria-label={es.app.searchTooltip}
        >
          <Search size={15} strokeWidth={2} />
        </button>
        <button
          type="button"
          className="pas-icon-btn"
          title={es.app.settingsTooltip}
          aria-label={es.app.settingsTooltip}
        >
          <Settings size={15} strokeWidth={2} />
        </button>
      </header>

      <div className={`pas-body${isDisconnected ? " is-dimmed" : ""}`}>
        <Sidebar
          groups={groups}
          projects={projects}
          sessions={sessions}
          activeSessionId={activeSessionId}
          onNewSession={() => openNewSessionModal()}
          onSelectSession={(sessionId) =>
            setLayout((prev) => openSessionTab(prev, sessionId))
          }
          onAddProject={() => setAddProjectOpen(true)}
          onRenameProject={(project) =>
            setTextPrompt({ kind: "rename-project", project })
          }
          onMoveProject={(projectId, groupId) => {
            void handleMoveProject(projectId, groupId);
          }}
          onCreateGroupAndMove={(project) =>
            setTextPrompt({ kind: "new-group-and-move", project })
          }
          onUnpinProject={(projectId) => {
            void handleUnpinProject(projectId);
          }}
          onRemoveProject={(project) => {
            void handleRemoveProject(project);
          }}
          onRenameGroup={(group) =>
            setTextPrompt({ kind: "rename-group", group })
          }
        />

        <main className="pas-main">
          <Tabs
            tabs={layout.tabs}
            activeTabId={layout.activeTabId}
            sessionsById={sessionsById}
            onSelectTab={(tabId) =>
              setLayout((prev) => ({ ...prev, activeTabId: tabId }))
            }
            onCloseTab={(tabId) =>
              setLayout((prev) => closeTab(prev, tabId))
            }
            onNewSession={() => openNewSessionModal()}
          />

          {layout.tabs.length === 0 ? (
            <div className="pas-empty-main">
              <h2>{es.tabs.emptyTitle}</h2>
              <p>{es.tabs.emptyBody}</p>
              <button
                type="button"
                className="pas-btn is-primary"
                style={{ display: "inline-flex", alignItems: "center", gap: 6 }}
                onClick={() => openNewSessionModal()}
              >
                <Plus size={14} strokeWidth={2.4} />
                <span>{es.tabs.emptyAction}</span>
              </button>
            </div>
          ) : (
            layout.tabs.map((tab) => {
              const isVisible = tab.id === layout.activeTabId;
              return (
                <div
                  key={tab.id}
                  style={{
                    flex: 1,
                    minHeight: 0,
                    display: isVisible ? "flex" : "none",
                  }}
                >
                  <SplitLayout
                    node={tab.root}
                    focusedSessionId={tab.focusedSessionId}
                    isVisible={isVisible}
                    focusTick={focusTick}
                    sessionsById={sessionsById}
                    onFocusSession={(sessionId) =>
                      setLayout((prev) => focusSession(prev, sessionId))
                    }
                    onSplitSession={(sessionId, dir) =>
                      openNewSessionModal({ dir, targetSessionId: sessionId })
                    }
                    onClosePane={(sessionId) =>
                      setLayout((prev) => closePane(prev, sessionId))
                    }
                    onResizeSplit={(path, sizes) =>
                      setLayout((prev) => resizeSplit(prev, tab.id, path, sizes))
                    }
                  />
                </div>
              );
            })
          )}
        </main>
      </div>

      {isDisconnected && (
        <div className="pas-banner" role="alert">
          <AlertCircle size={15} strokeWidth={2} />
          <span>
            {retryCountdown > 0
              ? es.disconnected.message(retryCountdown)
              : es.disconnected.messageNow}
          </span>
          <button
            type="button"
            className="pas-banner-retry"
            onClick={() => {
              setRetryCountdown(2);
              void api.reconnect().catch(() => {});
            }}
          >
            {es.disconnected.retryNow}
          </button>
        </div>
      )}

      {errorBanner && !isDisconnected && (
        <div className="pas-error-bar" role="alert">
          <AlertCircle size={15} strokeWidth={2} style={{ flex: "none", marginTop: 1 }} />
          <span>{errorBanner}</span>
          <button
            type="button"
            className="pas-pane-btn"
            style={{ color: "var(--banner-tx)" }}
            aria-label={es.errors.dismiss}
            onClick={() => setErrorBanner(null)}
          >
            <X size={13} />
          </button>
        </div>
      )}

      <footer className="pas-statusbar">
        <span className="pas-statusbar-conn">
          <span
            className={`pas-dot is-sm ${isDisconnected ? "is-err" : "is-ok"}`}
          />
          <span>
            {isDisconnected ? es.statusBar.disconnected : es.statusBar.connected}
          </span>
        </span>
        {!isDisconnected && (
          <span>{es.statusBar.sessionCount(sessions.length)}</span>
        )}
        <span className="pas-spacer" />
        {!isDisconnected && <span>{es.statusBar.shortcutsHint}</span>}
      </footer>

      <NewSessionDialog
        open={newSessionDialog.open}
        projects={projects}
        defaultProjectId={activeSession?.project_id ?? null}
        canSplit={activeSessionId !== null}
        defaultOpenInSplit={newSessionSplit.enabled}
        defaultSplitDir={newSessionSplit.dir}
        onClose={closeNewSessionModal}
        onSubmit={handleCreateSession}
      />

      <AddProjectDialog
        open={addProjectOpen}
        onClose={() => {
          setAddProjectOpen(false);
          setExtraFocusTick((n) => n + 1);
        }}
        onSubmit={handleAddProject}
      />

      <TextPromptDialog
        open={textPrompt !== null}
        title={
          textPrompt?.kind === "rename-project"
            ? es.renameDialog.projectTitle
            : textPrompt?.kind === "rename-group"
              ? es.renameDialog.groupTitle
              : es.renameDialog.newGroupTitle
        }
        initialValue={
          textPrompt?.kind === "rename-project"
            ? textPrompt.project.name
            : textPrompt?.kind === "rename-group"
              ? textPrompt.group.name
              : ""
        }
        submitLabel={
          textPrompt?.kind === "new-group-and-move"
            ? es.renameDialog.createSubmit
            : es.renameDialog.submit
        }
        onClose={() => {
          setTextPrompt(null);
          setExtraFocusTick((n) => n + 1);
        }}
        onSubmit={handleTextPromptSubmit}
      />
    </div>
  );
}
