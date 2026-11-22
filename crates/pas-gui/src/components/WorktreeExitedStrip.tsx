import type { WorktreeInfo, WorktreeStatus } from "../api/pas";
import { es } from "../i18n/es";
import {
  formatWorktreeBranchSummary,
  getExitedWorktreeActions,
} from "../state/worktree";

export type WorktreeStripBusy =
  | "reopening"
  | "leaving"
  | "removing"
  | "merging"
  | null;

interface WorktreeExitedStripProps {
  worktree: WorktreeInfo;
  status: WorktreeStatus | null;
  busyAction: WorktreeStripBusy;
  onReopen: () => void;
  onLeave: () => void;
  onRemoveOnly: () => void;
  onMergeAndRemove: () => void;
}

export function WorktreeExitedStrip({
  worktree,
  status,
  busyAction,
  onReopen,
  onLeave,
  onRemoveOnly,
  onMergeAndRemove,
}: WorktreeExitedStripProps) {
  const actions = getExitedWorktreeActions(status, worktree.state);
  const summaryText = status
    ? `${es.worktrees.exitedPrefix} ${formatWorktreeBranchSummary(worktree.base, status)}`
    : es.worktrees.exitedLoadingStatus;

  return (
    <div
      className="pas-pane-exited-bar is-worktree"
      role="status"
      data-worktree-strip={worktree.branch}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <span className="pas-pane-exited-msg">{summaryText}</span>
      <span className="pas-spacer" />
      <div className="pas-wt-strip-actions">
        {actions.showReopen && (
          <button
            type="button"
            className="pas-btn pas-btn-sm"
            disabled={busyAction !== null}
            onClick={(e) => {
              e.stopPropagation();
              onReopen();
            }}
          >
            {busyAction === "reopening" ? es.pane.reopening : es.pane.reopen}
          </button>
        )}
        {actions.showLeave && (
          <button
            type="button"
            className="pas-btn pas-btn-sm"
            disabled={busyAction !== null}
            onClick={(e) => {
              e.stopPropagation();
              onLeave();
            }}
          >
            {es.worktrees.leave}
          </button>
        )}
        {actions.showRemoveOnly && (
          <button
            type="button"
            className={`pas-btn pas-btn-sm${
              actions.primaryAction === "removeOnly" ? " is-primary" : ""
            }`}
            disabled={busyAction !== null || status === null}
            onClick={(e) => {
              e.stopPropagation();
              onRemoveOnly();
            }}
          >
            {busyAction === "removing"
              ? es.worktrees.removing
              : es.worktrees.removeOnly}
          </button>
        )}
        {actions.showMergeAndRemove && (
          <button
            type="button"
            className={`pas-btn pas-btn-sm${
              actions.primaryAction === "mergeAndRemove" ? " is-primary" : ""
            }`}
            disabled={busyAction !== null || status === null}
            onClick={(e) => {
              e.stopPropagation();
              onMergeAndRemove();
            }}
          >
            {busyAction === "merging"
              ? es.worktrees.merging
              : es.worktrees.mergeAndRemove}
          </button>
        )}
      </div>
    </div>
  );
}
