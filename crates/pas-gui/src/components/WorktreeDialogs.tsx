import { useEffect, useState } from "react";
import { GitBranch } from "lucide-react";
import type {
  ProjectInfo,
  SessionInfo,
  SessionState,
  WorktreeInfo,
  WorktreeListItem,
} from "../api/pas";
import { es } from "../i18n/es";
import {
  formatLossDetails,
  getWorktreeTableActions,
  getWorktreeTableStatusBadge,
} from "../state/worktree";

function sessionStateShort(state: SessionState): string {
  if (state.state === "running") return es.sessionState.running;
  if (state.state === "waiting") return es.sessionState.waiting;
  if (state.state === "idle") return es.sessionState.idle;
  return "terminó";
}

interface WorktreeRemoveConfirmDialogProps {
  item: WorktreeListItem | null;
  submitting: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => Promise<void>;
}

export function WorktreeRemoveConfirmDialog({
  item,
  submitting,
  error,
  onCancel,
  onConfirm,
}: WorktreeRemoveConfirmDialogProps) {
  const [confirmed, setConfirmed] = useState(false);

  useEffect(() => {
    setConfirmed(false);
  }, [item?.id, item]);

  useEffect(() => {
    if (!item) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !submitting) {
        e.preventDefault();
        e.stopPropagation();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [item, submitting, onCancel]);

  if (!item) return null;

  const details = formatLossDetails(item.base, item.status);

  return (
    <div
      className="pas-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="wt-confirm-remove-title"
      data-dialog="worktree-confirm-remove"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !submitting) onCancel();
      }}
    >
      <div className="pas-dialog-card is-worktree-confirm">
        <h4 id="wt-confirm-remove-title">
          {es.worktrees.confirmRemoveTitle(item.branch)}
        </h4>

        <div className="pas-wt-danger">
          <div>{es.worktrees.confirmRemoveIntro}</div>
          <div className="pas-wt-files">
            {details.uncommittedLabel && (
              <>
                <div>{details.uncommittedLabel}</div>
                {details.uncommittedFiles.map((file, idx) => (
                  <div key={`${file}-${idx}`} className="pas-wt-file-line">
                    {file}
                  </div>
                ))}
              </>
            )}
            {details.unmergedLabel && <div>{details.unmergedLabel}</div>}
          </div>
        </div>

        <label className="pas-check" htmlFor="wt-confirm-loss">
          <input
            id="wt-confirm-loss"
            type="checkbox"
            checked={confirmed}
            disabled={submitting}
            onChange={(e) => setConfirmed(e.target.checked)}
          />
          <span>{es.worktrees.confirmRemoveCheckbox}</span>
        </label>

        {error && <p className="pas-field-error">{error}</p>}

        <div className="pas-dialog-actions">
          <button
            type="button"
            className="pas-btn"
            disabled={submitting}
            onClick={onCancel}
          >
            {es.worktrees.confirmRemoveCancel}
          </button>
          <button
            type="button"
            className="pas-btn is-danger"
            disabled={!confirmed || submitting}
            onClick={() => {
              if (!confirmed) return;
              void onConfirm();
            }}
          >
            {submitting
              ? es.worktrees.removing
              : es.worktrees.confirmRemoveSubmit}
          </button>
        </div>
      </div>
    </div>
  );
}

interface WorktreeMergeErrorDialogProps {
  target: { worktree: WorktreeInfo; message: string } | null;
  onClose: () => void;
  onOpenSessionInWorktree: (worktree: WorktreeInfo) => void;
}

export function WorktreeMergeErrorDialog({
  target,
  onClose,
  onOpenSessionInWorktree,
}: WorktreeMergeErrorDialogProps) {
  useEffect(() => {
    if (!target) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [target, onClose]);

  if (!target) return null;

  const isConflict = target.message.toLowerCase().includes("conflict");

  return (
    <div
      className="pas-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="wt-merge-error-title"
      data-dialog="worktree-merge-error"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="pas-dialog-card is-worktree-confirm">
        <h4 id="wt-merge-error-title">
          {es.worktrees.mergeErrorTitle(target.worktree.branch)}
        </h4>

        <div className="pas-wt-warn">{target.message}</div>

        <p className="pas-wt-dialog-note">
          {isConflict
            ? es.worktrees.mergeErrorConflictHint(target.worktree.base)
            : es.worktrees.mergeErrorGenericHint}
        </p>

        <div className="pas-dialog-actions">
          <button
            type="button"
            className="pas-btn"
            onClick={() => onOpenSessionInWorktree(target.worktree)}
          >
            {es.worktrees.mergeErrorOpenWorktree}
          </button>
          <button
            type="button"
            className="pas-btn is-primary"
            onClick={onClose}
          >
            {es.worktrees.mergeErrorUnderstood}
          </button>
        </div>
      </div>
    </div>
  );
}

interface WorktreesDialogProps {
  project: ProjectInfo | null;
  items: WorktreeListItem[];
  loading: boolean;
  busyId: string | null;
  sessionsById: Map<string, SessionInfo>;
  suppressed?: boolean;
  onClose: () => void;
  onGoToSession: (sessionId: string) => void;
  onOpenSession: (item: WorktreeListItem) => void;
  onMerge: (item: WorktreeListItem) => Promise<void>;
  onRemove: (item: WorktreeListItem) => Promise<void>;
}

export function WorktreesDialog({
  project,
  items,
  loading,
  busyId,
  sessionsById,
  suppressed = false,
  onClose,
  onGoToSession,
  onOpenSession,
  onMerge,
  onRemove,
}: WorktreesDialogProps) {
  useEffect(() => {
    if (!project || suppressed) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [project, suppressed, onClose]);

  if (!project) return null;

  return (
    <div
      className="pas-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="wt-table-title"
      data-dialog="worktrees-list"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !suppressed) onClose();
      }}
    >
      <div className="pas-dialog-card is-worktrees">
        <h4 id="wt-table-title">{es.worktrees.tableTitle(project.name)}</h4>

        {loading && items.length === 0 ? (
          <p className="pas-wt-empty">{es.worktrees.tableLoading}</p>
        ) : items.length === 0 ? (
          <p className="pas-wt-empty">{es.worktrees.tableEmpty}</p>
        ) : (
          <div className="pas-wt-table-wrap">
            <table className="pas-wt-tbl">
              <thead>
                <tr>
                  <th>{es.worktrees.colBranch}</th>
                  <th>{es.worktrees.colStatus}</th>
                  <th>{es.worktrees.colSession}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {items.map((item) => {
                  const session = item.session_id
                    ? sessionsById.get(item.session_id)
                    : undefined;
                  const badge = getWorktreeTableStatusBadge(item);
                  const actions = getWorktreeTableActions(item, session);
                  const isBusy = busyId === item.id;
                  const sessionText = session
                    ? `${session.title} (${sessionStateShort(session.state)})`
                    : es.worktrees.noSession;

                  return (
                    <tr key={item.id} data-worktree-branch={item.branch}>
                      <td>
                        <span className="pas-br">
                          <GitBranch size={10} strokeWidth={2.2} />
                          <span>{item.branch}</span>
                        </span>
                      </td>
                      <td>
                        <span
                          className={`pas-wt-tag${
                            badge.tone === "ok"
                              ? " is-ok"
                              : badge.tone === "warn"
                                ? " is-warn"
                                : ""
                          }`}
                        >
                          {badge.label}
                        </span>
                      </td>
                      <td className="pas-wt-session-cell">{sessionText}</td>
                      <td className="pas-wt-actions-cell">
                        <div className="pas-wt-row-actions">
                          {actions.showGoToSession && session && (
                            <button
                              type="button"
                              className="pas-btn pas-btn-sm"
                              onClick={() => onGoToSession(session.id)}
                            >
                              {es.worktrees.goToSession}
                            </button>
                          )}
                          {actions.showOpenSession && (
                            <button
                              type="button"
                              className="pas-btn pas-btn-sm"
                              disabled={isBusy}
                              onClick={() => onOpenSession(item)}
                            >
                              {es.worktrees.openSession}
                            </button>
                          )}
                          {actions.showRemove && (
                            <button
                              type="button"
                              className="pas-btn pas-btn-sm"
                              disabled={isBusy}
                              onClick={() => void onRemove(item)}
                            >
                              {es.worktrees.remove}
                            </button>
                          )}
                          {actions.showMerge && (
                            <button
                              type="button"
                              className="pas-btn pas-btn-sm is-primary"
                              disabled={isBusy}
                              onClick={() => void onMerge(item)}
                            >
                              {es.worktrees.merge}
                            </button>
                          )}
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}

        <div className="pas-dialog-actions">
          <button
            type="button"
            className="pas-btn is-primary"
            onClick={onClose}
          >
            {es.worktrees.close}
          </button>
        </div>
      </div>
    </div>
  );
}
