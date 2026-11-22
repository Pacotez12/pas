import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionMode = "terminal" | "chat";

export type SessionState =
  | { state: "running" }
  | { state: "waiting" }
  | { state: "idle" }
  | { state: "exited"; code: number | null };

export type WorktreeState = "active" | "merged" | "removed";

export interface WorktreeInfo {
  id: string;
  project_id: string;
  branch: string;
  base: string;
  path: string;
  session_id: string | null;
  state: WorktreeState;
  created_at: number;
}

export interface WorktreeStatus {
  uncommitted: string[];
  ahead: number;
  merged: boolean;
  missing: boolean;
}

export interface WorktreeListItem extends WorktreeInfo {
  status: WorktreeStatus;
}

export interface WorktreeRequest {
  branch?: string | null;
  base?: string | null;
}

export interface WorktreeSuggestBranchResult {
  branch: string;
  base: string;
  path: string;
}

export interface WorktreeBranchesResult {
  current: string | null;
  branches: string[];
}

export interface SessionInfo {
  id: string;
  title: string;
  cwd: string;
  command: string[];
  state: SessionState;
  created_at: number;
  project_id: string | null;
  account_id: string | null;
  mode: SessionMode;
  worktree?: WorktreeInfo | null;
}

export type PermissionChoice = "allow" | "allow_session" | "deny";

export type ChatEvent =
  | { type: "user_message"; text: string }
  | { type: "agent_chunk"; text: string }
  | { type: "agent_message_end" }
  | { type: "tool_call"; id: string; title: string; status: string }
  | { type: "diff"; path: string; unified: string }
  | {
      type: "permission_request";
      request_id: string;
      title: string;
      detail?: string | null;
    }
  | {
      type: "permission_resolved";
      request_id: string;
      choice: PermissionChoice;
    }
  | { type: "visual"; file: string }
  | { type: "error"; message: string };

export interface ChatEventNotification {
  id: string;
  event: ChatEvent;
}

export interface AcpAdapterStatus {
  agent: AgentKind;
  bin: string;
  installed: boolean;
  path: string | null;
  package: string | null;
}

export type AgentKind = "claude" | "codex" | "agy";

export interface AccountInfo {
  id: string;
  agent: AgentKind;
  name: string;
  config_dir: string;
  owned: boolean;
  created_at: number;
  hooks_installed: boolean;
}

export interface QuotaWindow {
  used_percent: number;
  resets_at: number;
}

export interface Quota {
  five_h: QuotaWindow | null;
  week: QuotaWindow | null;
  updated_at: number;
}

export interface AccountQuota {
  account_id: string;
  quota: Quota | null;
}

export interface QuotaChangedEvent {
  account_id: string;
  quota: Quota;
}

export interface QuotaWarningEvent {
  account_id: string;
  window: "five_h" | "week";
  used_percent: number;
  resets_at: number;
  threshold_percent: number;
}

export interface SessionFocusRequestEvent {
  id: string;
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

export interface ClaudeConversation {
  id: string;
  title: string;
  modified_at: number;
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

export type OpenTarget = "editor" | "files" | "terminal";

export interface OpenExistsResult {
  exists: boolean;
  resolved?: string | null;
  is_dir: boolean;
}

export interface DiscoveredProject {
  name: string;
  path: string;
  remote_key?: string | null;
}

export interface DiscoveredAccount {
  agent: AgentKind;
  suggested_name: string;
  config_dir: string;
  already_added_as?: string | null;
}

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
    accountId?: string | null;
    command: string[];
    title: string | null;
    cwd: string;
    cols?: number;
    rows?: number;
    mode?: SessionMode;
    worktree?: WorktreeRequest | null;
  }) =>
    invoke<{ id: string }>("create_session", {
      projectId: params.projectId,
      accountId: params.accountId ?? null,
      command: params.command,
      title: params.title,
      cwd: params.cwd,
      cols: params.cols ?? null,
      rows: params.rows ?? null,
      mode: params.mode ?? "terminal",
      worktree: params.worktree ?? null,
    }),
  killSession: (id: string) => invoke<void>("kill_session", { id }),
  removeSession: (id: string) => invoke<void>("remove_session", { id }),
  claudeConversations: (cwd: string, accountId?: string | null) =>
    invoke<ClaudeConversation[]>("claude_conversations", {
      cwd,
      accountId: accountId ?? null,
    }),

  listAccounts: () => invoke<AccountInfo[]>("list_accounts"),
  addAccount: (params: {
    agent: AgentKind;
    name: string;
    configDir?: string | null;
  }) =>
    invoke<AccountInfo>("add_account", {
      agent: params.agent,
      name: params.name,
      configDir: params.configDir ?? null,
    }),
  renameAccount: (id: string, name: string) =>
    invoke<AccountInfo>("rename_account", { id, name }),
  removeAccount: (id: string, deleteDir: boolean) =>
    invoke<void>("remove_account", { id, deleteDir }),
  installAccountHooks: (id: string) =>
    invoke<AccountInfo>("install_account_hooks", { id }),
  uninstallAccountHooks: (id: string) =>
    invoke<AccountInfo>("uninstall_account_hooks", { id }),
  listQuotas: () => invoke<AccountQuota[]>("list_quotas"),
  defaultAgyDir: () => invoke<string>("default_agy_dir"),
  pasDataDir: () => invoke<string>("pas_data_dir"),

  acpStatus: () => invoke<AcpAdapterStatus[]>("acp_status"),
  acpInstall: (agent: AgentKind) =>
    invoke<AcpAdapterStatus>("acp_install", { agent }),

  chatPrompt: (id: string, text: string) =>
    invoke<void>("chat_prompt", { id, text }),
  chatPermission: (id: string, requestId: string, choice: PermissionChoice) =>
    invoke<void>("chat_permission", { id, requestId, choice }),
  chatCancel: (id: string) => invoke<void>("chat_cancel", { id }),
  chatHistory: (id: string) => invoke<ChatEvent[]>("chat_history", { id }),
  chatTerminalCommand: (id: string) =>
    invoke<string[] | null>("chat_terminal_command", { id }),
  chatReadVisual: (sessionId: string, file: string) =>
    invoke<string>("chat_read_visual", { sessionId, file }),
  chatOpenVisual: (sessionId: string, file: string) =>
    invoke<void>("chat_open_visual", { sessionId, file }),

  worktreeSuggestBranch: (params: {
    projectId: string;
    agent?: string | null;
    title?: string | null;
  }) =>
    invoke<WorktreeSuggestBranchResult>("worktree_suggest_branch", {
      projectId: params.projectId,
      agent: params.agent ?? null,
      title: params.title ?? null,
    }),
  worktreeBranches: (projectId: string) =>
    invoke<WorktreeBranchesResult>("worktree_branches", { projectId }),
  worktreeCreate: (params: {
    projectId: string;
    branch?: string | null;
    base?: string | null;
    agent?: string | null;
    title?: string | null;
  }) =>
    invoke<WorktreeInfo>("worktree_create", {
      projectId: params.projectId,
      branch: params.branch ?? null,
      base: params.base ?? null,
      agent: params.agent ?? null,
      title: params.title ?? null,
    }),
  worktreeList: (projectId: string) =>
    invoke<WorktreeListItem[]>("worktree_list", { projectId }),
  worktreeMerge: (id: string) => invoke<WorktreeInfo>("worktree_merge", { id }),
  worktreeRemove: (id: string, confirmLoss: boolean) =>
    invoke<void>("worktree_remove", { id, confirmLoss }),

  openPath: (path: string, target: OpenTarget, line?: number | null) =>
    invoke<void>("open_path", { path, target, line: line ?? null }),
  openExists: (cwd: string, path: string) =>
    invoke<OpenExistsResult>("open_exists", { cwd, path }),
  discoverProjects: () => invoke<DiscoveredProject[]>("discover_projects"),
  discoverAccounts: () => invoke<DiscoveredAccount[]>("discover_accounts"),

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
  onFocusRequest: (
    cb: (ev: SessionFocusRequestEvent) => void,
  ): Promise<UnlistenFn> =>
    listen<SessionFocusRequestEvent>("pas://focus_request", (e) => cb(e.payload)),
  onQuotaChanged: (cb: (ev: QuotaChangedEvent) => void): Promise<UnlistenFn> =>
    listen<QuotaChangedEvent>("pas://quota_changed", (e) => cb(e.payload)),
  onQuotaWarning: (cb: (ev: QuotaWarningEvent) => void): Promise<UnlistenFn> =>
    listen<QuotaWarningEvent>("pas://quota_warning", (e) => cb(e.payload)),
  onChatEvent: (cb: (ev: ChatEventNotification) => void): Promise<UnlistenFn> =>
    listen<ChatEventNotification>("pas://chat_event", (e) => cb(e.payload)),
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
