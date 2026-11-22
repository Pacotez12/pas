import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  Columns2,
  ExternalLink,
  GitBranch,
  Rows2,
  Square,
  X,
} from "lucide-react";
import {
  api,
  type AccountInfo,
  type ChatEvent,
  type PermissionChoice,
  type SessionInfo,
  type WorktreeInfo,
  type WorktreeStatus,
} from "../api/pas";
import { es } from "../i18n/es";
import {
  buildVisualSrcDoc,
  createInitialChatState,
  hydrateChatHistory,
  reduceChatEvent,
  type ChatItem,
  type ChatTimelineState,
} from "../state/chat";
import type { SplitDir } from "../state/layout";
import { statusDotClass } from "./Tabs";
import {
  WorktreeExitedStrip,
  type WorktreeStripBusy,
} from "./WorktreeExitedStrip";

interface ChatPaneProps {
  sessionId: string;
  session: SessionInfo | undefined;
  account: AccountInfo | undefined;
  worktreeStatus: WorktreeStatus | null;
  isFocused: boolean;
  isVisible: boolean;
  focusTick: number;
  onFocus: () => void;
  onSplit: (dir: SplitDir) => void;
  onClose: () => void;
  onOpenInTerminal: (session: SessionInfo, command: string[]) => Promise<void>;
  onRemoveExited: () => Promise<void>;
  onReopen: () => Promise<void>;
  onRemoveWorktree: (worktree: WorktreeInfo) => Promise<void>;
  onMergeAndRemoveWorktree: (worktree: WorktreeInfo) => Promise<void>;
}

function shortenPath(path: string): string {
  const match = path.match(/^\/home\/[^/]+(.*)$/);
  return match ? `~${match[1]}` : path;
}

function formatAgentLabel(
  session: SessionInfo | undefined,
  account: AccountInfo | undefined,
): string {
  if (account) {
    const label =
      account.agent === "claude"
        ? es.newSessionDialog.agents.claude
        : account.agent === "codex"
          ? es.newSessionDialog.agents.codex
          : es.newSessionDialog.agents.agy;
    return `${label} · ${account.name}`;
  }
  const first = session?.command[0] ?? "";
  const base = first.split("/").pop() ?? "";
  if (base === "claude" || base === "claude-agent-acp") {
    return es.newSessionDialog.agents.claude;
  }
  if (base === "codex" || base === "codex-acp") {
    return es.newSessionDialog.agents.codex;
  }
  if (base === "agy") {
    return es.newSessionDialog.agents.agy;
  }
  return session?.title || "chat";
}

function toolStatusText(status: string): string {
  switch (status) {
    case "pending":
      return es.chat.toolStatus.pending;
    case "in_progress":
      return es.chat.toolStatus.in_progress;
    case "completed":
      return es.chat.toolStatus.completed;
    case "failed":
      return es.chat.toolStatus.failed;
    default:
      return es.chat.toolStatus.unknown;
  }
}

function ToolRow({
  item,
}: {
  item: Extract<ChatItem, { kind: "tool" }>;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      className={`pas-chat-tool${expanded ? " is-expanded" : ""}`}
      data-tool-id={item.id}
    >
      <button
        type="button"
        className="pas-chat-tool-head"
        aria-expanded={expanded}
        onClick={() => setExpanded((v) => !v)}
      >
        {expanded ? (
          <ChevronDown size={13} strokeWidth={2} className="pas-chat-tool-chev" />
        ) : (
          <ChevronRight size={13} strokeWidth={2} className="pas-chat-tool-chev" />
        )}
        <span className="pas-chat-tool-title">{item.title || item.id}</span>
        <span className="pas-spacer" />
        <span
          className={`pas-chat-tool-status${
            item.status === "failed"
              ? " is-err"
              : item.status === "completed"
                ? " is-ok"
                : ""
          }`}
        >
          {toolStatusText(item.status)}
        </span>
      </button>
      {expanded && (
        <div className="pas-chat-tool-body">
          <span>
            {es.chat.toolIdLabel}: <code>{item.id}</code>
          </span>
          <span>
            {es.chat.toolStatusLabel}: <code>{item.status}</code>
          </span>
        </div>
      )}
    </div>
  );
}

function DiffCard({
  item,
}: {
  item: Extract<ChatItem, { kind: "diff" }>;
}) {
  const lines = item.unified
    .split("\n")
    .filter(
      (line, idx, arr) =>
        !(idx === arr.length - 1 && line === "") &&
        !line.startsWith("--- ") &&
        !line.startsWith("+++ "),
    );

  return (
    <div className="pas-chat-diff" data-diff-path={item.path}>
      <div className="pas-chat-diff-head">
        <span className="pas-chat-diff-path">{item.path}</span>
        <span className="pas-chat-diff-stats">
          +{item.additions} −{item.deletions}
        </span>
      </div>
      <div className="pas-chat-diff-body">
        {lines.map((line, i) => {
          const kindClass = line.startsWith("+")
            ? " is-add"
            : line.startsWith("-")
              ? " is-del"
              : line.startsWith("@@")
                ? " is-hunk"
                : "";
          return (
            <div key={i} className={`pas-chat-diff-line${kindClass}`}>
              {line}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function PermissionCard({
  item,
  busy,
  onResolve,
}: {
  item: Extract<ChatItem, { kind: "permission" }>;
  busy: boolean;
  onResolve: (requestId: string, choice: PermissionChoice) => void;
}) {
  const resolved = item.resolution !== null;

  return (
    <div
      className={`pas-chat-perm${resolved ? " is-resolved" : ""}`}
      data-permission-id={item.requestId}
      data-resolution={item.resolution ?? "pending"}
    >
      <div className="pas-chat-perm-title">{item.title}</div>
      {item.detail && <div className="pas-chat-mono">{item.detail}</div>}
      {resolved && item.resolution ? (
        <div className="pas-chat-perm-status">
          <Check size={12} strokeWidth={2.4} />
          <span>{es.chat.permissionResolved[item.resolution]}</span>
        </div>
      ) : (
        <div className="pas-chat-perm-actions">
          <button
            type="button"
            className="pas-btn pas-btn-sm is-primary"
            disabled={busy}
            onClick={() => onResolve(item.requestId, "allow")}
          >
            {es.chat.permissionAllow}
          </button>
          <button
            type="button"
            className="pas-btn pas-btn-sm"
            disabled={busy}
            onClick={() => onResolve(item.requestId, "allow_session")}
          >
            {es.chat.permissionAllowSession}
          </button>
          <button
            type="button"
            className="pas-btn pas-btn-sm"
            disabled={busy}
            onClick={() => onResolve(item.requestId, "deny")}
          >
            {es.chat.permissionDeny}
          </button>
        </div>
      )}
    </div>
  );
}

function VisualCard({
  sessionId,
  file,
  onOpenExternal,
}: {
  sessionId: string;
  file: string;
  onOpenExternal: (file: string) => void;
}) {
  const [srcDoc, setSrcDoc] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoadError(null);
    void api
      .chatReadVisual(sessionId, file)
      .then((html) => {
        if (!cancelled) {
          setSrcDoc(buildVisualSrcDoc(html));
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setLoadError(String(err));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, file]);

  return (
    <div className="pas-chat-vis" data-visual-file={file}>
      <div className="pas-chat-vis-head">
        <span className="pas-chat-vis-name">{file}</span>
        <span className="pas-spacer" />
        <button
          type="button"
          className="pas-chat-link"
          onClick={() => onOpenExternal(file)}
        >
          <ExternalLink size={11} strokeWidth={2} />
          <span>{es.chat.openInBrowser}</span>
        </button>
      </div>
      {loadError ? (
        <div className="pas-chat-vis-placeholder">
          {es.chat.visualError} {loadError}
        </div>
      ) : srcDoc === null ? (
        <div className="pas-chat-vis-placeholder">{es.chat.visualLoading}</div>
      ) : (
        <iframe
          className="pas-chat-vis-frame"
          sandbox="allow-scripts"
          srcDoc={srcDoc}
          title={file}
        />
      )}
    </div>
  );
}

export function ChatPane({
  sessionId,
  session,
  account,
  worktreeStatus,
  isFocused,
  isVisible,
  focusTick,
  onFocus,
  onSplit,
  onClose,
  onOpenInTerminal,
  onRemoveExited,
  onReopen,
  onRemoveWorktree,
  onMergeAndRemoveWorktree,
}: ChatPaneProps) {
  const [timeline, setTimeline] = useState<ChatTimelineState>(() =>
    createInitialChatState(),
  );
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [resolvingId, setResolvingId] = useState<string | null>(null);
  const [terminalCmd, setTerminalCmd] = useState<string[] | null>(null);
  const [openingTerminal, setOpeningTerminal] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<
    "closing" | WorktreeStripBusy
  >(null);

  const feedRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const pinnedToBottomRef = useRef(true);

  const stateKind = session?.state.state ?? "idle";
  const isRunning = stateKind === "running" || stateKind === "waiting";
  const isExited = stateKind === "exited";
  const exitedCode =
    session?.state.state === "exited" ? session.state.code : null;
  const activeWorktree =
    session?.worktree && session.worktree.state !== "removed"
      ? session.worktree
      : null;

  useEffect(() => {
    let disposed = false;
    let hydrated = false;
    const buffered: ChatEvent[] = [];

    setTimeline(createInitialChatState());
    setLocalError(null);

    let unlistenChat: (() => void) | null = null;
    let unlistenConn: (() => void) | null = null;

    const loadHistory = async (mergeBuffered: boolean) => {
      try {
        const history = await api.chatHistory(sessionId);
        if (disposed) return;
        const extra = mergeBuffered ? buffered.splice(0, buffered.length) : [];
        hydrated = true;
        setTimeline(hydrateChatHistory(history, extra));
      } catch {
        if (!disposed) {
          hydrated = true;
        }
      }
    };

    void (async () => {
      unlistenChat = await api.onChatEvent((ev) => {
        if (disposed || ev.id !== sessionId) return;
        if (!hydrated) {
          buffered.push(ev.event);
          return;
        }
        setTimeline((prev) => reduceChatEvent(prev, ev.event));
      });

      unlistenConn = await api.onConnection((status) => {
        if (disposed || status !== "connected") return;
        void loadHistory(false);
      });

      if (disposed) return;

      try {
        await api.subscribe(sessionId);
      } catch {
        // Bridge re-suscribirá al reconectar.
      }

      if (disposed) return;
      await loadHistory(true);
    })();

    return () => {
      disposed = true;
      unlistenChat?.();
      unlistenConn?.();
      void api.unsubscribe(sessionId).catch(() => {});
    };
  }, [sessionId]);

  useEffect(() => {
    let cancelled = false;
    void api
      .chatTerminalCommand(sessionId)
      .then((cmd) => {
        if (!cancelled) {
          setTerminalCmd(cmd && cmd.length > 0 ? cmd : null);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setTerminalCmd(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, stateKind]);

  useEffect(() => {
    const el = feedRef.current;
    if (!el || !pinnedToBottomRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [timeline]);

  useEffect(() => {
    if (!isVisible || !isFocused || isExited) return;
    const frame = window.requestAnimationFrame(() => {
      textareaRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [isVisible, isFocused, focusTick, isExited]);

  useEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 160)}px`;
  }, [draft]);

  async function handleSend() {
    const text = draft.trim();
    if (!text || sending || isRunning || isExited) return;
    setSending(true);
    setLocalError(null);
    pinnedToBottomRef.current = true;
    try {
      await api.chatPrompt(sessionId, draft);
      setDraft("");
    } catch (err) {
      setLocalError(
        es.errors.actionFailed(es.errors.actions.sendChatPrompt, String(err)),
      );
    } finally {
      setSending(false);
    }
  }

  async function handleCancel() {
    if (cancelling || !isRunning) return;
    setCancelling(true);
    setLocalError(null);
    try {
      await api.chatCancel(sessionId);
    } catch (err) {
      setLocalError(
        es.errors.actionFailed(es.errors.actions.cancelChat, String(err)),
      );
    } finally {
      setCancelling(false);
    }
  }

  async function handlePermission(requestId: string, choice: PermissionChoice) {
    if (resolvingId !== null) return;
    setResolvingId(requestId);
    setLocalError(null);
    try {
      await api.chatPermission(sessionId, requestId, choice);
    } catch (err) {
      setLocalError(
        es.errors.actionFailed(
          es.errors.actions.resolvePermission,
          String(err),
        ),
      );
    } finally {
      setResolvingId(null);
    }
  }

  async function handleOpenExternalVisual(file: string) {
    setLocalError(null);
    try {
      await api.chatOpenVisual(sessionId, file);
    } catch (err) {
      setLocalError(
        es.errors.actionFailed(es.errors.actions.openVisual, String(err)),
      );
    }
  }

  function handleComposerKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void handleSend();
    }
  }

  return (
    <div
      className={`pas-pane pas-chat-pane${isFocused ? " is-focused" : ""}`}
      data-session-id={sessionId}
      data-session-mode="chat"
      onMouseDown={() => onFocus()}
    >
      <div className="pas-pane-header">
        <span className={`pas-dot ${statusDotClass(session?.state)}`} />
        <span className="pas-pane-agent">
          {formatAgentLabel(session, account)}
        </span>
        {session?.cwd && (
          <>
            <span>·</span>
            <span className="pas-pane-cwd" title={session.cwd}>
              {shortenPath(session.cwd)}
            </span>
          </>
        )}
        {session?.worktree?.branch && (
          <span className="pas-br" title={session.worktree.branch}>
            <GitBranch size={10} strokeWidth={2.2} />
            <span>{session.worktree.branch}</span>
          </span>
        )}
        <span className="pas-spacer" />
        {terminalCmd && session && (
          <button
            type="button"
            className="pas-chat-link"
            disabled={openingTerminal}
            onClick={(e) => {
              e.stopPropagation();
              setOpeningTerminal(true);
              void onOpenInTerminal(session, terminalCmd).finally(() =>
                setOpeningTerminal(false),
              );
            }}
          >
            {openingTerminal
              ? es.chat.openingInTerminal
              : es.chat.openInTerminal}
          </button>
        )}
        <div className="pas-pane-actions">
          <button
            type="button"
            className="pas-pane-btn"
            title={es.pane.splitRightTooltip}
            aria-label={es.pane.splitRightTooltip}
            onClick={(e) => {
              e.stopPropagation();
              onSplit("row");
            }}
          >
            <Columns2 size={12} />
          </button>
          <button
            type="button"
            className="pas-pane-btn"
            title={es.pane.splitDownTooltip}
            aria-label={es.pane.splitDownTooltip}
            onClick={(e) => {
              e.stopPropagation();
              onSplit("col");
            }}
          >
            <Rows2 size={12} />
          </button>
          <button
            type="button"
            className="pas-pane-btn"
            title={es.pane.closePaneTooltip}
            aria-label={es.pane.closePaneTooltip}
            onClick={(e) => {
              e.stopPropagation();
              onClose();
            }}
          >
            <X size={12} />
          </button>
        </div>
      </div>

      <div
        ref={feedRef}
        className={`pas-chat-feed${isExited ? " is-exited" : ""}`}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinnedToBottomRef.current =
            el.scrollHeight - el.scrollTop - el.clientHeight < 36;
        }}
      >
        {timeline.items.length === 0 ? (
          <div className="pas-chat-empty">
            <p className="pas-chat-empty-title">{es.chat.emptyTitle}</p>
            <p className="pas-chat-empty-body">{es.chat.emptyBody}</p>
          </div>
        ) : (
          timeline.items.map((item) => {
            switch (item.kind) {
              case "user":
                return (
                  <div key={item.key} className="pas-chat-me">
                    {item.text}
                  </div>
                );
              case "agent":
                return (
                  <div
                    key={item.key}
                    className={`pas-chat-ai${item.streaming ? " is-streaming" : ""}`}
                  >
                    {item.text}
                  </div>
                );
              case "tool":
                return <ToolRow key={item.key} item={item} />;
              case "diff":
                return <DiffCard key={item.key} item={item} />;
              case "permission":
                return (
                  <PermissionCard
                    key={item.key}
                    item={item}
                    busy={resolvingId === item.requestId}
                    onResolve={(reqId, choice) =>
                      void handlePermission(reqId, choice)
                    }
                  />
                );
              case "visual":
                return (
                  <VisualCard
                    key={item.key}
                    sessionId={sessionId}
                    file={item.file}
                    onOpenExternal={(file) =>
                      void handleOpenExternalVisual(file)
                    }
                  />
                );
              case "error":
                return (
                  <div key={item.key} className="pas-chat-error" role="alert">
                    <AlertCircle size={14} strokeWidth={2} style={{ flex: "none", marginTop: 1 }} />
                    <span>{item.message}</span>
                  </div>
                );
            }
          })
        )}
      </div>

      {localError && (
        <div className="pas-chat-inline-error" role="alert">
          <AlertCircle size={13} strokeWidth={2} />
          <span>{localError}</span>
          <button
            type="button"
            className="pas-pane-btn"
            aria-label={es.errors.dismiss}
            onClick={() => setLocalError(null)}
          >
            <X size={12} />
          </button>
        </div>
      )}

      {isExited && activeWorktree ? (
        <WorktreeExitedStrip
          worktree={activeWorktree}
          status={worktreeStatus}
          busyAction={busyAction === "closing" ? "leaving" : busyAction}
          onReopen={() => {
            setBusyAction("reopening");
            void onReopen().finally(() => setBusyAction(null));
          }}
          onLeave={() => {
            setBusyAction("leaving");
            void onRemoveExited().finally(() => setBusyAction(null));
          }}
          onRemoveOnly={() => {
            setBusyAction("removing");
            void onRemoveWorktree(activeWorktree).finally(() =>
              setBusyAction(null),
            );
          }}
          onMergeAndRemove={() => {
            setBusyAction("merging");
            void onMergeAndRemoveWorktree(activeWorktree).finally(() =>
              setBusyAction(null),
            );
          }}
        />
      ) : isExited ? (
        <div
          className="pas-pane-exited-bar"
          role="status"
          onMouseDown={(e) => e.stopPropagation()}
        >
          <span className={`pas-dot ${statusDotClass(session?.state)}`} />
          <span className="pas-pane-exited-msg">
            {es.pane.exitedMessage(exitedCode)}
          </span>
          <span className="pas-spacer" />
          <button
            type="button"
            className="pas-btn pas-btn-sm"
            disabled={busyAction !== null}
            onClick={(e) => {
              e.stopPropagation();
              setBusyAction("closing");
              void onRemoveExited().finally(() => setBusyAction(null));
            }}
          >
            {es.pane.closeExited}
          </button>
          <button
            type="button"
            className="pas-btn pas-btn-sm is-primary"
            disabled={busyAction !== null}
            onClick={(e) => {
              e.stopPropagation();
              setBusyAction("reopening");
              void onReopen().finally(() => setBusyAction(null));
            }}
          >
            {busyAction === "reopening" ? es.pane.reopening : es.pane.reopen}
          </button>
        </div>
      ) : (
        <div
          className="pas-chat-composer"
          onMouseDown={(e) => e.stopPropagation()}
        >
          <textarea
            ref={textareaRef}
            className="pas-chat-ta"
            rows={1}
            value={draft}
            placeholder={es.chat.composerPlaceholder}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={handleComposerKeyDown}
          />
          <div className="pas-chat-cbar">
            <span className="pas-chat-hint">{es.chat.composerHint}</span>
            <span className="pas-spacer" />
            {isRunning && (
              <button
                type="button"
                className="pas-btn pas-btn-sm pas-btn-inline"
                disabled={cancelling}
                onClick={() => void handleCancel()}
              >
                <Square size={11} strokeWidth={2.2} />
                <span>
                  {cancelling ? es.chat.cancelling : es.chat.cancel}
                </span>
              </button>
            )}
            <button
              type="button"
              className="pas-btn pas-btn-sm is-primary"
              disabled={sending || isRunning || draft.trim().length === 0}
              onClick={() => void handleSend()}
            >
              {es.chat.send}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
