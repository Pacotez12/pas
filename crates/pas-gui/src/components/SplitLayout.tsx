import { Fragment, useRef } from "react";
import type { AccountInfo, SessionInfo } from "../api/pas";
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
  onFocusSession: (sessionId: string) => void;
  onSplitSession: (sessionId: string, dir: SplitDir) => void;
  onClosePane: (sessionId: string) => void;
  onOpenInTerminal: (session: SessionInfo, command: string[]) => Promise<void>;
  onRemoveExitedSession: (sessionId: string) => Promise<void>;
  onReopenSession: (sessionId: string) => Promise<void>;
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
  onFocusSession,
  onSplitSession,
  onClosePane,
  onOpenInTerminal,
  onRemoveExitedSession,
  onReopenSession,
  onResizeSplit,
}: SplitLayoutProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  if (node.kind === "pane") {
    const session = sessionsById.get(node.sessionId);
    if (session?.mode === "chat") {
      const account = session.account_id
        ? accountsById.get(session.account_id)
        : undefined;
      return (
        <ChatPane
          sessionId={node.sessionId}
          session={session}
          account={account}
          isFocused={node.sessionId === focusedSessionId}
          isVisible={isVisible}
          focusTick={focusTick}
          onFocus={() => onFocusSession(node.sessionId)}
          onSplit={(dir) => onSplitSession(node.sessionId, dir)}
          onClose={() => onClosePane(node.sessionId)}
          onOpenInTerminal={onOpenInTerminal}
          onRemoveExited={() => onRemoveExitedSession(node.sessionId)}
          onReopen={() => onReopenSession(node.sessionId)}
        />
      );
    }
    return (
      <TerminalPane
        sessionId={node.sessionId}
        session={session}
        isFocused={node.sessionId === focusedSessionId}
        isVisible={isVisible}
        focusTick={focusTick}
        onFocus={() => onFocusSession(node.sessionId)}
        onSplit={(dir) => onSplitSession(node.sessionId, dir)}
        onClose={() => onClosePane(node.sessionId)}
        onRemoveExited={() => onRemoveExitedSession(node.sessionId)}
        onReopen={() => onReopenSession(node.sessionId)}
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
                onFocusSession={onFocusSession}
                onSplitSession={onSplitSession}
                onClosePane={onClosePane}
                onOpenInTerminal={onOpenInTerminal}
                onRemoveExitedSession={onRemoveExitedSession}
                onReopenSession={onReopenSession}
                onResizeSplit={onResizeSplit}
              />
            </div>
          </Fragment>
        );
      })}
    </div>
  );
}
