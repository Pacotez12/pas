import type {
  SessionInfo,
  WorktreeListItem,
  WorktreeState,
  WorktreeStatus,
} from "../api/pas";

export interface ExitedWorktreeActions {
  showReopen: boolean;
  showLeave: boolean;
  showRemoveOnly: boolean;
  showMergeAndRemove: boolean;
  primaryAction: "mergeAndRemove" | "removeOnly" | "reopen" | "none";
}

export interface WorktreeStatusBadge {
  label: string;
  tone: "ok" | "warn" | "neutral";
}

export interface WorktreeTableActions {
  showGoToSession: boolean;
  showOpenSession: boolean;
  showRemove: boolean;
  showMerge: boolean;
}

export interface WorktreeLossDetails {
  uncommittedLabel: string | null;
  uncommittedFiles: string[];
  unmergedLabel: string | null;
}

function filesLabel(count: number): string {
  return count === 1
    ? "1 archivo sin commitear"
    : `${count} archivos sin commitear`;
}

function commitsLabel(count: number): string {
  return count === 1 ? "1 commit" : `${count} commits`;
}

function unmergedCommitsLabel(count: number): string {
  return count === 1
    ? "1 commit sin mergear"
    : `${count} commits sin mergear`;
}

export function worktreeHasLoss(status: WorktreeStatus): boolean {
  const hasUncommitted = status.uncommitted.length > 0;
  const hasUnmerged = status.ahead > 0 && !status.merged;
  return hasUncommitted || hasUnmerged;
}

export function formatWorktreeBranchSummary(
  base: string,
  status: WorktreeStatus,
): string {
  if (status.missing) {
    return "La carpeta del worktree ya no existe en disco.";
  }

  const uncommittedCount = status.uncommitted.length;
  if (status.merged) {
    if (uncommittedCount > 0) {
      return `La rama ya está mergeada en ${base}; ${filesLabel(uncommittedCount)}.`;
    }
    return `La rama ya está mergeada en ${base}; sin cambios sin commitear.`;
  }

  if (status.ahead > 0) {
    const verb = status.ahead === 1 ? "que no está en" : "que no están en";
    if (uncommittedCount > 0) {
      return `La rama tiene ${commitsLabel(status.ahead)} ${verb} ${base} y ${filesLabel(uncommittedCount)}.`;
    }
    return `La rama tiene ${commitsLabel(status.ahead)} ${verb} ${base}; sin cambios sin commitear.`;
  }

  if (uncommittedCount > 0) {
    return `Sin commits nuevos respecto de ${base}; ${filesLabel(uncommittedCount)}.`;
  }

  return `Sin commits nuevos ni cambios sin commitear respecto de ${base}.`;
}

export function getExitedWorktreeActions(
  status: WorktreeStatus | null,
  state: WorktreeState,
): ExitedWorktreeActions {
  if (state === "removed") {
    return {
      showReopen: false,
      showLeave: false,
      showRemoveOnly: false,
      showMergeAndRemove: false,
      primaryAction: "reopen",
    };
  }

  if (status?.missing) {
    return {
      showReopen: false,
      showLeave: true,
      showRemoveOnly: true,
      showMergeAndRemove: false,
      primaryAction: "removeOnly",
    };
  }

  const isMerged = state === "merged" || Boolean(status?.merged);
  const canMerge = !isMerged && (status === null || status.ahead > 0);
  const safeToRemove = status !== null && !worktreeHasLoss(status);
  const primaryAction = canMerge
    ? "mergeAndRemove"
    : isMerged && safeToRemove
      ? "removeOnly"
      : "none";

  return {
    showReopen: true,
    showLeave: true,
    showRemoveOnly: true,
    showMergeAndRemove: canMerge,
    primaryAction,
  };
}

export function getWorktreeTableStatusBadge(
  item: WorktreeListItem,
): WorktreeStatusBadge {
  const { status, state } = item;

  if (status.missing) {
    return { label: "carpeta borrada", tone: "warn" };
  }

  const uncommittedCount = status.uncommitted.length;
  const isMerged = state === "merged" || status.merged;
  const hasUnmerged = status.ahead > 0 && !isMerged;

  if (uncommittedCount > 0 && hasUnmerged) {
    return {
      label: `${filesLabel(uncommittedCount)} · ${unmergedCommitsLabel(status.ahead)}`,
      tone: "warn",
    };
  }

  if (uncommittedCount > 0) {
    return {
      label: filesLabel(uncommittedCount),
      tone: "warn",
    };
  }

  if (hasUnmerged) {
    return {
      label: unmergedCommitsLabel(status.ahead),
      tone: "warn",
    };
  }

  if (isMerged) {
    return { label: "mergeado", tone: "ok" };
  }

  return { label: "sin cambios", tone: "neutral" };
}

export function getWorktreeTableActions(
  item: WorktreeListItem,
  session: SessionInfo | undefined,
): WorktreeTableActions {
  const isSessionAlive = Boolean(session && session.state.state !== "exited");
  if (isSessionAlive) {
    return {
      showGoToSession: true,
      showOpenSession: false,
      showRemove: false,
      showMerge: false,
    };
  }

  if (item.status.missing) {
    return {
      showGoToSession: false,
      showOpenSession: false,
      showRemove: true,
      showMerge: false,
    };
  }

  const isMerged = item.state === "merged" || item.status.merged;
  const hasUncommitted = item.status.uncommitted.length > 0;
  const canMerge = !isMerged && item.status.ahead > 0;

  return {
    showGoToSession: false,
    showOpenSession: !isMerged || hasUncommitted,
    showRemove: true,
    showMerge: canMerge,
  };
}

export function formatLossDetails(
  base: string,
  status: WorktreeStatus,
): WorktreeLossDetails {
  const uncommittedCount = status.uncommitted.length;
  const hasUnmerged = status.ahead > 0 && !status.merged;
  const verb = status.ahead === 1 ? "que no está en" : "que no están en";

  return {
    uncommittedLabel:
      uncommittedCount > 0 ? filesLabel(uncommittedCount) : null,
    uncommittedFiles: uncommittedCount > 0 ? [...status.uncommitted] : [],
    unmergedLabel: hasUnmerged
      ? `${commitsLabel(status.ahead)} ${verb} ${base}`
      : null,
  };
}

export function isConfirmLossError(errorText: string): boolean {
  return (
    errorText.includes("Marcá la casilla") ||
    errorText.includes("1024")
  );
}

export function computeWorktreePreviewPath(
  suggestedPath: string,
  suggestedBranch: string,
  editedBranch: string,
): string {
  const trimmed = editedBranch.trim();
  if (!trimmed || !suggestedPath) {
    return suggestedPath;
  }
  const sanitize = (b: string) => b.replace(/[/\\]/g, "-");
  const oldSegment = sanitize(suggestedBranch);
  const newSegment = sanitize(trimmed);
  if (oldSegment && suggestedPath.endsWith(`/${oldSegment}`)) {
    return `${suggestedPath.slice(0, suggestedPath.length - oldSegment.length)}${newSegment}`;
  }
  const lastSlash = suggestedPath.lastIndexOf("/");
  if (lastSlash >= 0) {
    return `${suggestedPath.slice(0, lastSlash + 1)}${newSegment}`;
  }
  return suggestedPath;
}
