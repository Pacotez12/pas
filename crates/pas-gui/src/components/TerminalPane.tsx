import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import { Columns2, GitBranch, Rows2, X } from "lucide-react";
import {
  api,
  decodeBase64Bytes,
  encodeUtf8Base64,
  type SessionInfo,
  type WorktreeInfo,
  type WorktreeStatus,
} from "../api/pas";
import { es } from "../i18n/es";
import type { SplitDir } from "../state/layout";
import { statusDotClass } from "./Tabs";
import {
  WorktreeExitedStrip,
  type WorktreeStripBusy,
} from "./WorktreeExitedStrip";

interface TerminalPaneProps {
  sessionId: string;
  session: SessionInfo | undefined;
  worktreeStatus: WorktreeStatus | null;
  isFocused: boolean;
  isVisible: boolean;
  focusTick: number;
  onFocus: () => void;
  onSplit: (dir: SplitDir) => void;
  onClose: () => void;
  onRemoveExited: () => Promise<void>;
  onReopen: () => Promise<void>;
  onRemoveWorktree: (worktree: WorktreeInfo) => Promise<void>;
  onMergeAndRemoveWorktree: (worktree: WorktreeInfo) => Promise<void>;
}

function isPasShortcut(ev: KeyboardEvent): boolean {
  if (ev.type !== "keydown") return false;
  if (ev.ctrlKey && ev.shiftKey && !ev.altKey && !ev.metaKey) {
    const key = ev.key.toLowerCase();
    if (key === "t" || key === "d" || key === "e" || key === "w") {
      return true;
    }
  }
  if (ev.ctrlKey && !ev.shiftKey && !ev.altKey && !ev.metaKey) {
    if (ev.key === "PageUp" || ev.key === "PageDown") {
      return true;
    }
  }
  return false;
}

function shortenPath(path: string): string {
  const match = path.match(/^\/home\/[^/]+(.*)$/);
  return match ? `~${match[1]}` : path;
}

function agentName(session: SessionInfo | undefined): string {
  if (!session) return "sesión";
  const first = session.command[0];
  if (first) {
    const parts = first.split("/");
    return parts[parts.length - 1] || session.title;
  }
  return session.title;
}

export function TerminalPane({
  sessionId,
  session,
  worktreeStatus,
  isFocused,
  isVisible,
  focusTick,
  onFocus,
  onSplit,
  onClose,
  onRemoveExited,
  onReopen,
  onRemoveWorktree,
  onMergeAndRemoveWorktree,
}: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [dims, setDims] = useState<{ cols: number; rows: number }>({
    cols: 120,
    rows: 40,
  });
  const [busyAction, setBusyAction] = useState<
    "closing" | WorktreeStripBusy
  >(null);

  const isExited = session?.state.state === "exited";
  const exitedCode = session?.state.state === "exited" ? session.state.code : null;
  const activeWorktree =
    session?.worktree && session.worktree.state !== "removed"
      ? session.worktree
      : null;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      fontFamily: '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 12.5,
      lineHeight: 1.45,
      cursorBlink: true,
      scrollback: 5000,
      theme: {
        background: "#0e0e12",
        foreground: "#c8c8d2",
        cursor: "#c8c8d2",
        cursorAccent: "#0e0e12",
        selectionBackground: "rgba(242, 207, 120, 0.22)",
        black: "#15151b",
        red: "#ef6b6b",
        green: "#5fd38d",
        yellow: "#f2cf78",
        blue: "#7aa2f7",
        magenta: "#bb9af7",
        cyan: "#7dcfff",
        white: "#c8c8d2",
        brightBlack: "#666678",
        brightRed: "#f78c8c",
        brightGreen: "#7ee3a6",
        brightYellow: "#f7de9a",
        brightBlue: "#9bb8fa",
        brightMagenta: "#cfb7fa",
        brightCyan: "#9ee0ff",
        brightWhite: "#ffffff",
      },
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(host);

    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
      });
      term.loadAddon(webgl);
    } catch {
      // Fallback automático al renderer DOM de xterm.
    }

    term.attachCustomKeyEventHandler((ev) => !isPasShortcut(ev));

    termRef.current = term;
    fitRef.current = fitAddon;

    const hostWithReader = host as HTMLDivElement & {
      __pasReadBuffer?: () => string;
    };
    hostWithReader.__pasReadBuffer = () => {
      const buf = term.buffer.active;
      const lines: string[] = [];
      for (let i = 0; i < buf.length; i += 1) {
        const line = buf.getLine(i);
        if (line) lines.push(line.translateToString(true));
      }
      return lines.join("\n");
    };

    let inputChain = Promise.resolve();
    const dataDisposable = term.onData((data) => {
      const encoded = encodeUtf8Base64(data);
      inputChain = inputChain
        .then(() => api.sessionInput(sessionId, encoded))
        .catch(() => {});
    });

    let disposed = false;
    let ready = false;
    let resizeTimer: number | null = null;
    let lastCols = 0;
    let lastRows = 0;

    const scheduleResize = () => {
      if (disposed) return;
      try {
        fitAddon.fit();
      } catch {
        return;
      }
      const cols = term.cols;
      const rows = term.rows;
      if (cols <= 0 || rows <= 0) return;
      setDims((prev) =>
        prev.cols === cols && prev.rows === rows ? prev : { cols, rows },
      );
      if (cols === lastCols && rows === lastRows) return;
      lastCols = cols;
      lastRows = rows;
      if (resizeTimer !== null) {
        window.clearTimeout(resizeTimer);
      }
      resizeTimer = window.setTimeout(() => {
        resizeTimer = null;
        if (!disposed) {
          void api.sessionResize(sessionId, cols, rows).catch(() => {});
        }
      }, 50);
    };

    const observer = new ResizeObserver(() => {
      scheduleResize();
    });
    observer.observe(host);
    scheduleResize();

    let unlistenOutput: (() => void) | null = null;
    let unlistenSnapshot: (() => void) | null = null;

    void (async () => {
      unlistenOutput = await api.onOutput((ev) => {
        if (disposed || !ready || ev.id !== sessionId) return;
        term.write(decodeBase64Bytes(ev.data_b64));
      });
      unlistenSnapshot = await api.onSnapshot((ev) => {
        if (disposed || ev.id !== sessionId) return;
        term.reset();
        term.write(decodeBase64Bytes(ev.data_b64));
      });

      if (disposed) return;

      try {
        await api.subscribe(sessionId);
      } catch {
        // Si pasd está reconectando, Bridge re-suscribirá al volver.
      }

      if (disposed) return;

      try {
        const snap = await api.sessionSnapshot(sessionId);
        if (!disposed && snap.data_b64) {
          term.reset();
          term.write(decodeBase64Bytes(snap.data_b64));
        }
      } catch {
        // Sesión recién creada o desconectada.
      }

      ready = true;
    })();

    return () => {
      disposed = true;
      if (resizeTimer !== null) {
        window.clearTimeout(resizeTimer);
      }
      observer.disconnect();
      dataDisposable.dispose();
      unlistenOutput?.();
      unlistenSnapshot?.();
      void api.unsubscribe(sessionId).catch(() => {});
      delete hostWithReader.__pasReadBuffer;
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [sessionId]);

  useEffect(() => {
    if (!isVisible) return;
    const fit = fitRef.current;
    const term = termRef.current;
    if (!fit || !term) return;
    const frame = window.requestAnimationFrame(() => {
      try {
        fit.fit();
        setDims({ cols: term.cols, rows: term.rows });
        if (isFocused) {
          term.focus();
        }
      } catch {
        // Contenedor todavía sin dimensiones.
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [isVisible, isFocused, focusTick]);

  return (
    <div
      className={`pas-pane${isFocused ? " is-focused" : ""}`}
      data-session-id={sessionId}
      onMouseDown={() => {
        onFocus();
        termRef.current?.focus();
      }}
    >
      <div className="pas-pane-header">
        <span className={`pas-dot ${statusDotClass(session?.state)}`} />
        <span className="pas-pane-agent">{agentName(session)}</span>
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
        <span className="pas-pane-dims">
          {dims.cols}×{dims.rows}
        </span>
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
        ref={hostRef}
        className={`pas-terminal-host${isExited ? " is-exited" : ""}`}
      />
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
      ) : null}
    </div>
  );
}
