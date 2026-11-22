export interface DialogState {
  open: boolean;
  focusTick: number;
}

export type DialogKeyAction = "close-dropdown" | "close-dialog" | "ignore";

export function initialDialogState(): DialogState {
  return { open: false, focusTick: 0 };
}

export function openDialog(state: DialogState): DialogState {
  return { ...state, open: true };
}

export function closeDialog(state: DialogState): DialogState {
  if (!state.open) return state;
  return { open: false, focusTick: state.focusTick + 1 };
}

export function resolveDialogKey(
  key: string,
  dropdownOpen: boolean,
): DialogKeyAction {
  if (key !== "Escape") return "ignore";
  return dropdownOpen ? "close-dropdown" : "close-dialog";
}
