import { describe, expect, it } from "vitest";
import {
  closeDialog,
  initialDialogState,
  openDialog,
  resolveDialogKey,
} from "./dialog";

describe("state/dialog", () => {
  it("closing dialog increments focusTick so the active pane regains focus", () => {
    let state = initialDialogState();
    expect(state.open).toBe(false);
    expect(state.focusTick).toBe(0);

    state = openDialog(state);
    expect(state.open).toBe(true);
    expect(state.focusTick).toBe(0);

    state = closeDialog(state);
    expect(state.open).toBe(false);
    expect(state.focusTick).toBe(1);

    // Closing again when already closed does not bump focusTick unnecessarily.
    const unchanged = closeDialog(state);
    expect(unchanged.focusTick).toBe(1);
  });

  it("Escape closes open dropdown first, then closes dialog", () => {
    expect(resolveDialogKey("Escape", true)).toBe("close-dropdown");
    expect(resolveDialogKey("Escape", false)).toBe("close-dialog");
    expect(resolveDialogKey("Enter", false)).toBe("ignore");
  });
});
