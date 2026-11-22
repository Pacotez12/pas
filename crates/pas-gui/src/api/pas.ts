import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionState =
  | { state: "running" }
  | { state: "idle" }
  | { state: "exited"; code: number | null };

export interface SessionInfo {
  id: string;
  title: string;
  cwd: string;
  command: string[];
  state: SessionState;
  created_at: number;
  project_id: string | null;
}

export interface ProjectInfo {
  id: string;
  name: string;
  path: string;
  group_id: string;
  group_pinned: boolean;
}

export interface GroupInfo {
  id: string;
  name: string;
  remote_key: string | null;
}

export interface SessionOutputEvent {
  id: string;
  data_b64: string;
}

export interface SessionStateEvent {
  id: string;
  state: SessionState;
}

export interface SessionCreatedEvent {
  info: SessionInfo;
}

export interface SessionRemovedEvent {
  id: string;
}

export type ConnectionStatus = "connected" | "disconnected";

export const api = {
  listProjects: () => invoke<ProjectInfo[]>("list_projects"),
  addProject: (path: string, name?: string) =>
    invoke<ProjectInfo>("add_project", { path, name: name || null }),
  removeProject: (id: string) => invoke<void>("remove_project", { id }),
  renameProject: (id: string, name: string) =>
    invoke<ProjectInfo>("rename_project", { id, name }),
  moveProject: (id: string, groupId: string) =>
    invoke<ProjectInfo>("move_project", { id, groupId }),
  unpinProject: (id: string) => invoke<ProjectInfo>("unpin_project", { id }),

  listGroups: () => invoke<GroupInfo[]>("list_groups"),
  createGroup: (name: string) => invoke<GroupInfo>("create_group", { name }),
  renameGroup: (id: string, name: string) =>
    invoke<GroupInfo>("rename_group", { id, name }),

  listSessions: () => invoke<SessionInfo[]>("list_sessions"),
  createSession: (params: {
    projectId: string | null;
    command: string[];
    title: string | null;
    cwd: string;
    cols?: number;
    rows?: number;
  }) =>
    invoke<{ id: string }>("create_session", {
      projectId: params.projectId,
      command: params.command,
      title: params.title,
      cwd: params.cwd,
      cols: params.cols ?? null,
      rows: params.rows ?? null,
    }),
  killSession: (id: string) => invoke<void>("kill_session", { id }),
  sessionInput: (id: string, dataB64: string) =>
    invoke<void>("session_input", { id, dataB64 }),
  sessionResize: (id: string, cols: number, rows: number) =>
    invoke<void>("session_resize", { id, cols, rows }),
  sessionSnapshot: (id: string) =>
    invoke<{ data_b64: string }>("session_snapshot", { id }),
  subscribe: (id: string) => invoke<void>("subscribe", { id }),
  unsubscribe: (id: string) => invoke<void>("unsubscribe", { id }),
  reconnect: () => invoke<void>("reconnect"),
  loadLayout: () => invoke<string | null>("load_layout"),
  saveLayout: (json: string) => invoke<void>("save_layout", { json }),

  onOutput: (cb: (ev: SessionOutputEvent) => void): Promise<UnlistenFn> =>
    listen<SessionOutputEvent>("pas://output", (e) => cb(e.payload)),
  onSnapshot: (cb: (ev: SessionOutputEvent) => void): Promise<UnlistenFn> =>
    listen<SessionOutputEvent>("pas://snapshot", (e) => cb(e.payload)),
  onState: (cb: (ev: SessionStateEvent) => void): Promise<UnlistenFn> =>
    listen<SessionStateEvent>("pas://state", (e) => cb(e.payload)),
  onCreated: (cb: (ev: SessionCreatedEvent) => void): Promise<UnlistenFn> =>
    listen<SessionCreatedEvent>("pas://created", (e) => cb(e.payload)),
  onRemoved: (cb: (ev: SessionRemovedEvent) => void): Promise<UnlistenFn> =>
    listen<SessionRemovedEvent>("pas://removed", (e) => cb(e.payload)),
  onConnection: (cb: (status: ConnectionStatus) => void): Promise<UnlistenFn> =>
    listen<ConnectionStatus>("pas://connection", (e) => cb(e.payload)),
};

export function encodeUtf8Base64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export function decodeBase64Bytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
