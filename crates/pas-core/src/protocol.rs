use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const METHOD_DAEMON_PING: &str = "daemon.ping";
pub const METHOD_SESSION_CREATE: &str = "session.create";
pub const METHOD_SESSION_LIST: &str = "session.list";
pub const METHOD_SESSION_SEND: &str = "session.send";
pub const METHOD_SESSION_READ: &str = "session.read";
pub const METHOD_SESSION_WAIT: &str = "session.wait";
pub const METHOD_SESSION_KILL: &str = "session.kill";
pub const METHOD_SESSION_REMOVE: &str = "session.remove";
pub const METHOD_SESSION_INPUT: &str = "session.input";
pub const METHOD_SESSION_RESIZE: &str = "session.resize";
pub const METHOD_SESSION_SNAPSHOT: &str = "session.snapshot";
pub const METHOD_SESSION_SUBSCRIBE: &str = "session.subscribe";
pub const METHOD_SESSION_UNSUBSCRIBE: &str = "session.unsubscribe";
pub const METHOD_SESSION_SUBSCRIBE_ALL: &str = "session.subscribe_all";
pub const METHOD_SESSION_HOOK: &str = "session.hook";
pub const METHOD_PROJECT_ADD: &str = "project.add";
pub const METHOD_PROJECT_LIST: &str = "project.list";
pub const METHOD_PROJECT_REMOVE: &str = "project.remove";
pub const METHOD_PROJECT_RENAME: &str = "project.rename";
pub const METHOD_PROJECT_MOVE: &str = "project.move";
pub const METHOD_PROJECT_UNPIN: &str = "project.unpin";
pub const METHOD_GROUP_LIST: &str = "group.list";
pub const METHOD_GROUP_CREATE: &str = "group.create";
pub const METHOD_GROUP_RENAME: &str = "group.rename";
pub const METHOD_ACCOUNT_LIST: &str = "account.list";
pub const METHOD_ACCOUNT_ADD: &str = "account.add";
pub const METHOD_ACCOUNT_RENAME: &str = "account.rename";
pub const METHOD_ACCOUNT_REMOVE: &str = "account.remove";
pub const METHOD_ACCOUNT_INSTALL_HOOKS: &str = "account.install_hooks";
pub const METHOD_ACCOUNT_UNINSTALL_HOOKS: &str = "account.uninstall_hooks";
pub const METHOD_QUOTA_REPORT: &str = "quota.report";
pub const METHOD_QUOTA_LIST: &str = "quota.list";
pub const METHOD_CLAUDE_CONVERSATIONS: &str = "claude.conversations";
pub const METHOD_CHAT_PROMPT: &str = "chat.prompt";
pub const METHOD_CHAT_PERMISSION: &str = "chat.permission";
pub const METHOD_CHAT_CANCEL: &str = "chat.cancel";
pub const METHOD_CHAT_HISTORY: &str = "chat.history";
pub const METHOD_CHAT_TERMINAL_COMMAND: &str = "chat.terminal_command";
pub const METHOD_ACP_STATUS: &str = "acp.status";
pub const METHOD_ACP_INSTALL: &str = "acp.install";
pub const METHOD_WORKTREE_CREATE: &str = "worktree.create";
pub const METHOD_WORKTREE_LIST: &str = "worktree.list";
pub const METHOD_WORKTREE_MERGE: &str = "worktree.merge";
pub const METHOD_WORKTREE_REMOVE: &str = "worktree.remove";
pub const METHOD_WORKTREE_SUGGEST_BRANCH: &str = "worktree.suggest_branch";

pub const EVENT_SESSION_OUTPUT: &str = "session.output";
pub const EVENT_SESSION_STATE: &str = "session.state";
pub const EVENT_SESSION_CREATED: &str = "session.created";
pub const EVENT_SESSION_REMOVED: &str = "session.removed";
pub const EVENT_SESSION_FOCUS_REQUEST: &str = "session.focus_request";
pub const EVENT_QUOTA_CHANGED: &str = "quota.changed";
pub const EVENT_QUOTA_WARNING: &str = "quota.warning";
pub const EVENT_CHAT_EVENT: &str = "chat.event";

pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERR_INVALID_PARAMS: i32 = -32602;
pub const ERR_SESSION_NOT_FOUND: i32 = 1001;
pub const ERR_SPAWN_FAILED: i32 = 1002;
pub const ERR_WAIT_TIMEOUT: i32 = 1003;
pub const ERR_SESSION_EXITED: i32 = 1004;
pub const ERR_PROJECT_NOT_FOUND: i32 = 1005;
pub const ERR_PROJECT_ALREADY_EXISTS: i32 = 1006;
pub const ERR_NOT_A_GIT_REPO: i32 = 1007;
pub const ERR_GROUP_NOT_FOUND: i32 = 1008;
pub const ERR_GROUP_ALREADY_EXISTS: i32 = 1009;
pub const ERR_SESSION_RUNNING: i32 = 1010;
pub const ERR_ACCOUNT_NOT_FOUND: i32 = 1011;
pub const ERR_ACCOUNT_ALREADY_EXISTS: i32 = 1012;
pub const ERR_ACCOUNT_DIR_INVALID: i32 = 1013;
pub const ERR_HOOKS_CONFIG_INVALID: i32 = 1014;
pub const ERR_WRONG_SESSION_MODE: i32 = 1015;
pub const ERR_PERMISSION_NOT_FOUND: i32 = 1016;
pub const ERR_CHAT_BUSY: i32 = 1017;
pub const ERR_ADAPTER_NOT_FOUND: i32 = 1018;
pub const ERR_ADAPTER_INSTALL_FAILED: i32 = 1019;
pub const ERR_WORKTREE_NOT_FOUND: i32 = 1020;
pub const ERR_WORKTREE_DIRTY_MAIN: i32 = 1021;
pub const ERR_WORKTREE_WRONG_BASE: i32 = 1022;
pub const ERR_WORKTREE_MERGE_CONFLICT: i32 = 1023;
pub const ERR_WORKTREE_CONFIRM_REQUIRED: i32 = 1024;
pub const ERR_WORKTREE_BRANCH_EXISTS: i32 = 1025;
pub const ERR_WORKTREE_GIT_FAILED: i32 = 1026;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum SessionState {
    Running,
    Waiting,
    Idle,
    Exited { code: Option<i32> },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Terminal,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub command: Vec<String>,
    pub state: SessionState,
    pub created_at: u64,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub worktree: Option<WorktreeInfo>,
}

fn default_cols() -> u16 {
    120
}

fn default_rows() -> u16 {
    40
}

fn default_enter() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateParams {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub worktree: Option<WorktreeRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendParams {
    pub id: String,
    pub text: String,
    #[serde(default = "default_enter")]
    pub enter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadParams {
    pub id: String,
    #[serde(default)]
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResult {
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WaitUntil {
    Idle,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitParams {
    pub id: String,
    pub until: WaitUntil,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingResult {
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateResult {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputParams {
    pub id: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeParams {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResult {
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAddParams {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub group_id: String,
    #[serde(default)]
    pub group_pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRenameParams {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMoveParams {
    pub id: String,
    pub group_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub remote_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupCreateParams {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRenameParams {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOutputEvent {
    pub id: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStateEvent {
    pub id: String,
    pub state: SessionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCreatedEvent {
    pub info: SessionInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRemovedEvent {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFocusRequestEvent {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeConversationsParams {
    pub cwd: PathBuf,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeConversation {
    pub id: String,
    pub title: String,
    pub modified_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Agy,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Agy => "agy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "agy" => Some(Self::Agy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: String,
    pub agent: AgentKind,
    pub name: String,
    pub config_dir: PathBuf,
    pub owned: bool,
    pub created_at: u64,
    #[serde(default)]
    pub hooks_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAddParams {
    pub agent: AgentKind,
    pub name: String,
    #[serde(default)]
    pub config_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRenameParams {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRemoveParams {
    pub id: String,
    #[serde(default)]
    pub delete_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookParams {
    pub id: String,
    pub agent: AgentKind,
    pub event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaReportParams {
    pub agent: AgentKind,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub used_percent: f32,
    pub resets_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    #[serde(default)]
    pub five_h: Option<QuotaWindow>,
    #[serde(default)]
    pub week: Option<QuotaWindow>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountQuota {
    pub account_id: String,
    #[serde(default)]
    pub quota: Option<Quota>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaChangedEvent {
    pub account_id: String,
    pub quota: Quota,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaWindowKind {
    FiveH,
    Week,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWarningEvent {
    pub account_id: String,
    pub window: QuotaWindowKind,
    pub used_percent: f32,
    pub resets_at: i64,
    pub threshold_percent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionChoice {
    Allow,
    AllowSession,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatPromptParams {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatPermissionParams {
    pub id: String,
    pub request_id: String,
    pub choice: PermissionChoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    UserMessage {
        text: String,
    },
    AgentChunk {
        text: String,
    },
    AgentMessageEnd,
    ToolCall {
        id: String,
        title: String,
        status: String,
    },
    Diff {
        path: String,
        unified: String,
    },
    PermissionRequest {
        request_id: String,
        title: String,
        #[serde(default)]
        detail: Option<String>,
    },
    PermissionResolved {
        request_id: String,
        choice: PermissionChoice,
    },
    Visual {
        file: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEventNotification {
    pub id: String,
    pub event: ChatEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpAdapterStatus {
    pub agent: AgentKind,
    pub bin: String,
    pub installed: bool,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub package: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpInstallParams {
    pub agent: AgentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeState {
    Active,
    Merged,
    Removed,
}

impl WorktreeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Merged => "merged",
            Self::Removed => "removed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "merged" => Some(Self::Merged),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub id: String,
    pub project_id: String,
    pub branch: String,
    pub base: String,
    pub path: PathBuf,
    #[serde(default)]
    pub session_id: Option<String>,
    pub state: WorktreeState,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeStatus {
    pub uncommitted: Vec<String>,
    pub ahead: u32,
    pub merged: bool,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeListItem {
    #[serde(flatten)]
    pub info: WorktreeInfo,
    pub status: WorktreeStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRequest {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeCreateParams {
    pub project_id: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeListParams {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRemoveParams {
    pub id: String,
    #[serde(default)]
    pub confirm_loss: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeSuggestBranchParams {
    pub project_id: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeSuggestBranchResult {
    pub branch: String,
    pub base: String,
    pub path: PathBuf,
}
