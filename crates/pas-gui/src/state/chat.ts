import type { ChatEvent, PermissionChoice } from "../api/pas";

export type ChatItem =
  | { kind: "user"; key: string; text: string }
  | { kind: "agent"; key: string; text: string; streaming: boolean }
  | { kind: "tool"; key: string; id: string; title: string; status: string }
  | {
      kind: "diff";
      key: string;
      path: string;
      unified: string;
      additions: number;
      deletions: number;
    }
  | {
      kind: "permission";
      key: string;
      requestId: string;
      title: string;
      detail: string | null;
      resolution: PermissionChoice | null;
    }
  | { kind: "visual"; key: string; file: string }
  | { kind: "error"; key: string; message: string };

export interface ChatTimelineState {
  items: ChatItem[];
  seq: number;
}

export function createInitialChatState(): ChatTimelineState {
  return { items: [], seq: 0 };
}

function closeOpenAgentMessage(items: ChatItem[]): ChatItem[] {
  if (items.length === 0) return items;
  const last = items[items.length - 1];
  if (last.kind === "agent" && last.streaming) {
    return [...items.slice(0, -1), { ...last, streaming: false }];
  }
  return items;
}

function countDiffChanges(unified: string): {
  additions: number;
  deletions: number;
} {
  let additions = 0;
  let deletions = 0;
  for (const line of unified.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) {
      additions += 1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      deletions += 1;
    }
  }
  return { additions, deletions };
}

export function reduceChatEvent(
  state: ChatTimelineState,
  event: ChatEvent,
): ChatTimelineState {
  const nextSeq = state.seq + 1;

  switch (event.type) {
    case "user_message": {
      const items = closeOpenAgentMessage(state.items);
      return {
        seq: nextSeq,
        items: [
          ...items,
          { kind: "user", key: `u-${nextSeq}`, text: event.text },
        ],
      };
    }

    case "agent_chunk": {
      if (event.text.length === 0) {
        return { ...state, seq: nextSeq };
      }
      const last = state.items[state.items.length - 1];
      if (last && last.kind === "agent" && last.streaming) {
        return {
          seq: nextSeq,
          items: [
            ...state.items.slice(0, -1),
            { ...last, text: last.text + event.text },
          ],
        };
      }
      return {
        seq: nextSeq,
        items: [
          ...state.items,
          {
            kind: "agent",
            key: `a-${nextSeq}`,
            text: event.text,
            streaming: true,
          },
        ],
      };
    }

    case "agent_message_end": {
      return {
        seq: nextSeq,
        items: closeOpenAgentMessage(state.items),
      };
    }

    case "tool_call": {
      const items = closeOpenAgentMessage(state.items);
      const existingIdx = items.findIndex(
        (item) => item.kind === "tool" && item.id === event.id,
      );
      if (existingIdx >= 0) {
        const existing = items[existingIdx];
        if (existing.kind === "tool") {
          const updated: ChatItem = {
            ...existing,
            title: event.title || existing.title,
            status: event.status,
          };
          return {
            seq: nextSeq,
            items: [
              ...items.slice(0, existingIdx),
              updated,
              ...items.slice(existingIdx + 1),
            ],
          };
        }
      }
      return {
        seq: nextSeq,
        items: [
          ...items,
          {
            kind: "tool",
            key: `t-${event.id || nextSeq}`,
            id: event.id,
            title: event.title,
            status: event.status,
          },
        ],
      };
    }

    case "diff": {
      const items = closeOpenAgentMessage(state.items);
      const { additions, deletions } = countDiffChanges(event.unified);
      return {
        seq: nextSeq,
        items: [
          ...items,
          {
            kind: "diff",
            key: `d-${nextSeq}`,
            path: event.path,
            unified: event.unified,
            additions,
            deletions,
          },
        ],
      };
    }

    case "permission_request": {
      const items = closeOpenAgentMessage(state.items);
      const already = items.some(
        (item) =>
          item.kind === "permission" && item.requestId === event.request_id,
      );
      if (already) {
        return { seq: nextSeq, items };
      }
      return {
        seq: nextSeq,
        items: [
          ...items,
          {
            kind: "permission",
            key: `p-${event.request_id}`,
            requestId: event.request_id,
            title: event.title,
            detail: event.detail ?? null,
            resolution: null,
          },
        ],
      };
    }

    case "permission_resolved": {
      const idx = state.items.findIndex(
        (item) =>
          item.kind === "permission" && item.requestId === event.request_id,
      );
      if (idx < 0) {
        return { ...state, seq: nextSeq };
      }
      const target = state.items[idx];
      if (target.kind !== "permission") {
        return { ...state, seq: nextSeq };
      }
      return {
        seq: nextSeq,
        items: [
          ...state.items.slice(0, idx),
          { ...target, resolution: event.choice },
          ...state.items.slice(idx + 1),
        ],
      };
    }

    case "visual": {
      const items = closeOpenAgentMessage(state.items);
      const already = items.some(
        (item) => item.kind === "visual" && item.file === event.file,
      );
      if (already) {
        return { seq: nextSeq, items };
      }
      return {
        seq: nextSeq,
        items: [
          ...items,
          { kind: "visual", key: `v-${event.file}`, file: event.file },
        ],
      };
    }

    case "error": {
      const items = closeOpenAgentMessage(state.items);
      return {
        seq: nextSeq,
        items: [
          ...items,
          { kind: "error", key: `e-${nextSeq}`, message: event.message },
        ],
      };
    }
  }
}

function chatEventEquals(a: ChatEvent, b: ChatEvent): boolean {
  if (a.type !== b.type) return false;
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Reconstruye el timeline desde `chat.history` y fusiona los eventos en vivo
 * recibidos mientras `chat.history` estaba en vuelo, eliminando cualquier
 * solapamiento por sufijo/prefijo para no duplicar ni perder eventos.
 */
export function hydrateChatHistory(
  history: ChatEvent[],
  buffered: ChatEvent[] = [],
): ChatTimelineState {
  let overlap = 0;
  const maxOverlap = Math.min(history.length, buffered.length);
  for (let k = maxOverlap; k >= 1; k -= 1) {
    let matches = true;
    for (let i = 0; i < k; i += 1) {
      if (!chatEventEquals(history[history.length - k + i], buffered[i])) {
        matches = false;
        break;
      }
    }
    if (matches) {
      overlap = k;
      break;
    }
  }

  let state = createInitialChatState();
  for (const ev of history) {
    state = reduceChatEvent(state, ev);
  }
  for (let i = overlap; i < buffered.length; i += 1) {
    state = reduceChatEvent(state, buffered[i]);
  }
  return state;
}

const VISUAL_CSP_META =
  '<meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'; script-src \'unsafe-inline\'; img-src data:;">';

export function buildVisualSrcDoc(html: string): string {
  const headMatch = /<head(\s[^>]*)?>/i.exec(html);
  if (headMatch) {
    const insertAt = headMatch.index + headMatch[0].length;
    return `${html.slice(0, insertAt)}${VISUAL_CSP_META}${html.slice(insertAt)}`;
  }
  return `<!doctype html><html><head><meta charset="utf-8">${VISUAL_CSP_META}</head><body>${html}</body></html>`;
}
