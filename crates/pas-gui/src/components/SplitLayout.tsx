import { Fragment, useRef } from "react";
import type {
  AccountInfo,
  OpenTarget,
  SessionInfo,
  WorktreeInfo,
  WorktreeStatus,
} from "../api/pas";
import type { LayoutNode, SplitDir } from "../state/layout";
import { ChatPane } from "./ChatPane";
import { TerminalPane } from "./TerminalPane";

interface SplitLayoutProps {
  node: LayoutNode;
  path?: number[];
  focusedSessionId: string;
  isVisible: boolean;
  focusTick: number;
  sessionsById: Map<string, SessionInfo>;
  accountsById: Map<string, AccountInfo>;
  worktreeStatusById: Map<string, WorktreeStatus>;
  onFocusSession: (sessionId: string) => void;
  onSplitSession: (sessionId: string, dir: SplitDir) => void;
  onClosePane: (sessionId: string) => void;
  onOpenInTerminal: (session: SessionInfo, command: string[]) => Promise<void>;
  onOpenPath: (path: string, target: OpenTarget, line?: number | null) => void;
  onOpenUrl: (url: string) => void;
  onCopyPath: (path: string) => void;
  onRemoveExitedSession: (sessionId: string) => Promise<void>;
  onReopenSession: (sessionId: string) => Promise<void>;
  onRemoveWorktree: (
    worktree: WorktreeInfo,
    exitedSessionId: string,
  ) => Promise<void>;
  onMergeAndRemoveWorktree: (
    worktree: WorktreeInfo,
    exitedSessionId: string,
  ) => Promise<void>;
  onResizeSplit: (path: number[], sizes: number[]) => void;
}

export function SplitLayout({
  node,
  path = [],
  focusedSessionId,
  isVisible,
  focusTick,
  sessionsById,
  accountsById,
  worktreeStatusById,
  onFocusSession,
  onSplitSession,
  onClosePane,
  onOpenInTerminal,
  onOpenPath,
  onOpenUrl,
  onCopyPath,
  onRemoveExitedSession,
  onReopenSession,
  onRemoveWorktree,
  onMergeAndRemoveWorktree,
  onResizeSplit,
}: SplitLayoutProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  if (node.kind === "pane") {
    const session = sessionsById.get(node.sessionId);
    const worktreeStatus = session?.worktree
      ? worktreeStatusById.get(session.worktree.id) ?? null
      : null;
    if (session?.mode === "chat") {
      const account = session.account_id
        ? accountsById.get(session.account_id)
        : undefined;
      return (
        <ChatPane
          sessionId={node.sessionId}
          session={session}
          account={account}
          worktreeStatus={worktreeStatus}
          isFocused={node.sessionId === focusedSessionId}
          isVisible={isVisible}
          focusTick={focusTick}
          onFocus={() => onFocusSession(node.sessionId)}
          onSplit={(dir) => onSplitSession(node.sessionId, dir)}
          onClose={() => onClosePane(node.sessionId)}
          onOpenInTerminal={onOpenInTerminal}
          onOpenPath={onOpenPath}
          onOpenUrl={onOpenUrl}
          onCopyPath={onCopyPath}
          onRemoveExited={() => onRemoveExitedSession(node.sessionId)}
          onReopen={() => onReopenSession(node.sessionId)}
          onRemoveWorktree={(wt) => onRemoveWorktree(wt, node.sessionId)}
          onMergeAndRemoveWorktree={(wt) =>
            onMergeAndRemoveWorktree(wt, node.sessionId)
          }
        />
      );
    }
    return (
      <TerminalPane
        sessionId={node.sessionId}
        session={session}
        worktreeStatus={worktreeStatus}
        isFocused={node.sessionId === focusedSessionId}
        isVisible={isVisible}
        focusTick={focusTick}
        onFocus={() => onFocusSession(node.sessionId)}
        onSplit={(dir) => onSplitSession(node.sessionId, dir)}
        onClose={() => onClosePane(node.sessionId)}
        onOpenPath={onOpenPath}
        onOpenUrl={onOpenUrl}
        onCopyPath={onCopyPath}
        onRemoveExited={() => onRemoveExitedSession(node.sessionId)}
        onReopen={() => onReopenSession(node.sessionId)}
        onRemoveWorktree={(wt) => onRemoveWorktree(wt, node.sessionId)}
        onMergeAndRemoveWorktree={(wt) =>
          onMergeAndRemoveWorktree(wt, node.sessionId)
        }
      />
    );
  }

  function startDrag(dividerIdx: number, startEvent: React.PointerEvent<HTMLDivElement>) {
    if (node.kind !== "split") return;
    const container = containerRef.current;
    if (!container) return;

    startEvent.preventDefault();
    startEvent.currentTarget.setPointerCapture(startEvent.pointerId);

    const rect = container.getBoundingClientRect();
    const totalPx = node.dir === "row" ? rect.width : rect.height;
    if (totalPx <= 0) return;

    const startPos = node.dir === "row" ? startEvent.clientX : startEvent.clientY;
    const initialSizes = [...node.sizes];
    const leftInit = initialSizes[dividerIdx];
    const rightInit = initialSizes[dividerIdx + 1];
    const pairSum = leftInit + rightInit;

    const onMove = (ev: PointerEvent) => {
      const currentPos = node.dir === "row" ? ev.clientX : ev.clientY;
      const deltaPct = ((currentPos - startPos) / totalPx) * 100;
      const minPct = 15;
      const nextLeft = Math.max(minPct, Math.min(pairSum - minPct, leftInit + deltaPct));
      const nextRight = pairSum - nextLeft;
      const updated = [...initialSizes];
      updated[dividerIdx] = nextLeft;
      updated[dividerIdx + 1] = nextRight;
      onResizeSplit(path, updated);
    };

    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  return (
    <div
      ref={containerRef}
      className={`pas-split ${node.dir === "row" ? "is-row" : "is-col"}`}
    >
      {node.children.map((child, idx) => {
        const size = node.sizes[idx] ?? 100 / node.children.length;
        return (
          <Fragment key={idx}>
            {idx > 0 && (
              <div
                className={`pas-split-divider ${node.dir === "row" ? "is-row" : "is-col"}`}
                onPointerDown={(e) => startDrag(idx - 1, e)}
              />
            )}
            <div
              className="pas-split-child"
              style={{ flex: `${size} 1 0%` }}
            >
              <SplitLayout
                node={child}
                path={[...path, idx]}
                focusedSessionId={focusedSessionId}
                isVisible={isVisible}
                focusTick={focusTick}
                sessionsById={sessionsById}
                accountsById={accountsById}
                worktreeStatusById={worktreeStatusById}
                onFocusSession={onFocusSession}
                onSplitSession={onSplitSession}
                onClosePane={onClosePane}
                onOpenInTerminal={onOpenInTerminal}
                onOpenPath={onOpenPath}
                onOpenUrl={onOpenUrl}
                onCopyPath={onCopyPath}
                onRemoveExitedSession={onRemoveExitedSession}
                onReopenSession={onReopenSession}
                onRemoveWorktree={onRemoveWorktree}
                onMergeAndRemoveWorktree={onMergeAndRemoveWorktree}
                onResizeSplit={onResizeSplit}
              />
            </div>
          </Fragment>
        );
      })}
    </div>
  );
}
