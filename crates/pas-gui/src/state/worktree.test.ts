import { describe, expect, it } from "vitest";
import type { SessionInfo, WorktreeListItem, WorktreeStatus } from "../api/pas";
import {
  computeWorktreePreviewPath,
  formatLossDetails,
  formatWorktreeBranchSummary,
  getExitedWorktreeActions,
  getWorktreeTableActions,
  getWorktreeTableStatusBadge,
  isConfirmLossError,
  worktreeHasLoss,
} from "./worktree";

function makeStatus(overrides: Partial<WorktreeStatus> = {}): WorktreeStatus {
  return {
    uncommitted: [],
    ahead: 0,
    merged: false,
    missing: false,
    ...overrides,
  };
}

function makeItem(
  overrides: Partial<WorktreeListItem> = {},
  statusOverrides: Partial<WorktreeStatus> = {},
): WorktreeListItem {
  return {
    id: "wt-1",
    project_id: "proj-1",
    branch: "pas/agy-exportar-csv",
    base: "main",
    path: "/tmp/wt/pockt/pas-agy-exportar-csv",
    session_id: null,
    state: "active",
    created_at: 1_700_000_000,
    status: makeStatus(statusOverrides),
    ...overrides,
  };
}

function makeSession(state: SessionInfo["state"]): SessionInfo {
  return {
    id: "ses-1",
    title: "agy · exportar-csv",
    cwd: "/tmp/wt/pockt/pas-agy-exportar-csv",
    command: ["agy"],
    state,
    created_at: 1_700_000_000,
    project_id: "proj-1",
    account_id: null,
    mode: "terminal",
    worktree: null,
  };
}

describe("state/worktree", () => {
  describe("worktreeHasLoss", () => {
    it("is false when clean and no unmerged commits", () => {
      expect(worktreeHasLoss(makeStatus())).toBe(false);
    });

    it("is false when already merged and no uncommitted files", () => {
      expect(worktreeHasLoss(makeStatus({ ahead: 3, merged: true }))).toBe(false);
    });

    it("is true when uncommitted files exist even if merged", () => {
      expect(
        worktreeHasLoss(
          makeStatus({ uncommitted: [" M lib/export.dart"], merged: true }),
        ),
      ).toBe(true);
    });

    it("is true when there are unmerged commits ahead of base", () => {
      expect(worktreeHasLoss(makeStatus({ ahead: 2, merged: false }))).toBe(true);
    });
  });

  describe("formatWorktreeBranchSummary", () => {
    it("formats multiple unmerged commits with clean working tree", () => {
      const summary = formatWorktreeBranchSummary(
        "main",
        makeStatus({ ahead: 3 }),
      );
      expect(summary).toBe(
        "La rama tiene 3 commits que no están en main; sin cambios sin commitear.",
      );
    });

    it("formats singular commit and uncommitted files", () => {
      const summary = formatWorktreeBranchSummary(
        "main",
        makeStatus({ ahead: 1, uncommitted: [" M a.ts", "?? b.ts"] }),
      );
      expect(summary).toBe(
        "La rama tiene 1 commit que no está en main y 2 archivos sin commitear.",
      );
    });

    it("formats uncommitted files without new commits", () => {
      const summary = formatWorktreeBranchSummary(
        "main",
        makeStatus({ ahead: 0, uncommitted: [" M a.ts"] }),
      );
      expect(summary).toBe(
        "Sin commits nuevos respecto de main; 1 archivo sin commitear.",
      );
    });

    it("formats clean branch with no commits ahead", () => {
      const summary = formatWorktreeBranchSummary("main", makeStatus());
      expect(summary).toBe(
        "Sin commits nuevos ni cambios sin commitear respecto de main.",
      );
    });

    it("formats merged branch without uncommitted files", () => {
      const summary = formatWorktreeBranchSummary(
        "main",
        makeStatus({ merged: true }),
      );
      expect(summary).toBe(
        "La rama ya está mergeada en main; sin cambios sin commitear.",
      );
    });

    it("formats missing worktree folder", () => {
      const summary = formatWorktreeBranchSummary(
        "main",
        makeStatus({ missing: true }),
      );
      expect(summary).toBe("La carpeta del worktree ya no existe en disco.");
    });
  });

  describe("getExitedWorktreeActions", () => {
    it("offers merge and remove as primary when unmerged commits exist", () => {
      const actions = getExitedWorktreeActions(
        makeStatus({ ahead: 3, merged: false }),
        "active",
      );
      expect(actions).toEqual({
        showReopen: true,
        showLeave: true,
        showRemoveOnly: true,
        showMergeAndRemove: true,
        primaryAction: "mergeAndRemove",
      });
    });

    it("hides merge and keeps remove neutral when there are no commits ahead and not merged", () => {
      const actions = getExitedWorktreeActions(
        makeStatus({ ahead: 0, uncommitted: [" M a.ts"] }),
        "active",
      );
      expect(actions).toEqual({
        showReopen: true,
        showLeave: true,
        showRemoveOnly: true,
        showMergeAndRemove: false,
        primaryAction: "none",
      });
    });

    it("hides merge and keeps remove neutral when branch is ancestor of base but has uncommitted files", () => {
      const actions = getExitedWorktreeActions(
        makeStatus({ ahead: 0, merged: true, uncommitted: ["?? pendiente.txt"] }),
        "active",
      );
      expect(actions).toEqual({
        showReopen: true,
        showLeave: true,
        showRemoveOnly: true,
        showMergeAndRemove: false,
        primaryAction: "none",
      });
    });

    it("hides merge when branch is already merged", () => {
      const actions = getExitedWorktreeActions(
        makeStatus({ ahead: 0, merged: true }),
        "merged",
      );
      expect(actions).toEqual({
        showReopen: true,
        showLeave: true,
        showRemoveOnly: true,
        showMergeAndRemove: false,
        primaryAction: "removeOnly",
      });
    });

    it("hides reopen and merge when worktree folder is missing", () => {
      const actions = getExitedWorktreeActions(
        makeStatus({ missing: true }),
        "active",
      );
      expect(actions).toEqual({
        showReopen: false,
        showLeave: true,
        showRemoveOnly: true,
        showMergeAndRemove: false,
        primaryAction: "removeOnly",
      });
    });

    it("hides all worktree actions when state is removed", () => {
      const actions = getExitedWorktreeActions(makeStatus(), "removed");
      expect(actions).toEqual({
        showReopen: false,
        showLeave: false,
        showRemoveOnly: false,
        showMergeAndRemove: false,
        primaryAction: "reopen",
      });
    });
  });

  describe("getWorktreeTableStatusBadge", () => {
    it("shows unmerged commits badge in warn tone", () => {
      const badge = getWorktreeTableStatusBadge(makeItem({}, { ahead: 3 }));
      expect(badge).toEqual({ label: "3 commits sin mergear", tone: "warn" });
    });

    it("shows merged badge in ok tone when clean", () => {
      const badge = getWorktreeTableStatusBadge(
        makeItem({ state: "merged" }, { merged: true }),
      );
      expect(badge).toEqual({ label: "mergeado", tone: "ok" });
    });

    it("shows uncommitted badge in warn tone", () => {
      const badge = getWorktreeTableStatusBadge(
        makeItem({}, { uncommitted: [" M a.ts"] }),
      );
      expect(badge).toEqual({
        label: "1 archivo sin commitear",
        tone: "warn",
      });
    });

    it("combines uncommitted and unmerged counts", () => {
      const badge = getWorktreeTableStatusBadge(
        makeItem({}, { ahead: 2, uncommitted: [" M a.ts", "?? b.ts"] }),
      );
      expect(badge).toEqual({
        label: "2 archivos sin commitear · 2 commits sin mergear",
        tone: "warn",
      });
    });

    it("shows missing badge when folder was deleted manually", () => {
      const badge = getWorktreeTableStatusBadge(
        makeItem({}, { missing: true }),
      );
      expect(badge).toEqual({ label: "carpeta borrada", tone: "warn" });
    });

    it("shows neutral badge when clean with no commits", () => {
      const badge = getWorktreeTableStatusBadge(makeItem());
      expect(badge).toEqual({ label: "sin cambios", tone: "neutral" });
    });
  });

  describe("getWorktreeTableActions", () => {
    it("shows only go-to-session while linked session is active", () => {
      const actions = getWorktreeTableActions(
        makeItem({ session_id: "ses-1" }, { uncommitted: [" M a.ts"] }),
        makeSession({ state: "running" }),
      );
      expect(actions).toEqual({
        showGoToSession: true,
        showOpenSession: false,
        showRemove: false,
        showMerge: false,
      });
    });

    it("shows open session, remove and merge when session exited with unmerged commits", () => {
      const actions = getWorktreeTableActions(
        makeItem({ session_id: "ses-1" }, { ahead: 3 }),
        makeSession({ state: "exited", code: 0 }),
      );
      expect(actions).toEqual({
        showGoToSession: false,
        showOpenSession: true,
        showRemove: true,
        showMerge: true,
      });
    });

    it("shows only remove for merged clean worktree without session", () => {
      const actions = getWorktreeTableActions(
        makeItem({ state: "merged" }, { merged: true }),
        undefined,
      );
      expect(actions).toEqual({
        showGoToSession: false,
        showOpenSession: false,
        showRemove: true,
        showMerge: false,
      });
    });

    it("shows only remove when folder is missing", () => {
      const actions = getWorktreeTableActions(
        makeItem({}, { missing: true }),
        undefined,
      );
      expect(actions).toEqual({
        showGoToSession: false,
        showOpenSession: false,
        showRemove: true,
        showMerge: false,
      });
    });
  });

  describe("formatLossDetails", () => {
    it("lists uncommitted files and unmerged commits", () => {
      const details = formatLossDetails(
        "main",
        makeStatus({
          uncommitted: [" M lib/export.dart", "?? test/export_test.dart"],
          ahead: 3,
          merged: false,
        }),
      );
      expect(details).toEqual({
        uncommittedLabel: "2 archivos sin commitear",
        uncommittedFiles: [" M lib/export.dart", "?? test/export_test.dart"],
        unmergedLabel: "3 commits que no están en main",
      });
    });

    it("omits unmerged label when branch is already merged", () => {
      const details = formatLossDetails(
        "main",
        makeStatus({
          uncommitted: [" M lib/export.dart"],
          ahead: 2,
          merged: true,
        }),
      );
      expect(details).toEqual({
        uncommittedLabel: "1 archivo sin commitear",
        uncommittedFiles: [" M lib/export.dart"],
        unmergedLabel: null,
      });
    });
  });

  describe("isConfirmLossError", () => {
    it("detects confirm-loss messages from pasd", () => {
      expect(
        isConfirmLossError(
          "No se puede borrar el worktree pas/agy-x: tiene 2 archivos sin commitear. Marcá la casilla para confirmar que se pierden o commitealos primero.",
        ),
      ).toBe(true);
    });

    it("returns false for unrelated errors", () => {
      expect(
        isConfirmLossError(
          "No se pudo mergear pas/agy-x porque hay conflictos en: lib/a.dart.",
        ),
      ).toBe(false);
    });
  });

  describe("computeWorktreePreviewPath", () => {
    it("updates folder name when user edits branch name", () => {
      expect(
        computeWorktreePreviewPath(
          "/home/u/.local/share/pas/worktrees/pockt/pas-agy-exportar",
          "pas/agy-exportar",
          "pas/agy-exportar-csv",
        ),
      ).toBe("/home/u/.local/share/pas/worktrees/pockt/pas-agy-exportar-csv");
    });

    it("returns suggested path when edited branch is empty", () => {
      expect(
        computeWorktreePreviewPath(
          "/home/u/.local/share/pas/worktrees/pockt/pas-agy-exportar",
          "pas/agy-exportar",
          "   ",
        ),
      ).toBe("/home/u/.local/share/pas/worktrees/pockt/pas-agy-exportar");
    });
  });
});
