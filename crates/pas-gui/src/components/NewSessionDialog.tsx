import { useEffect, useRef, useState, type FormEvent } from "react";
import { Check, ChevronDown, FolderPlus } from "lucide-react";
import {
  api,
  type AccountInfo,
  type ClaudeConversation,
  type ProjectInfo,
  type Quota,
} from "../api/pas";
import { es } from "../i18n/es";
import { filterAccountsByAgent, formatQuotaInline } from "../state/accounts";
import { resolveDialogKey } from "../state/dialog";
import type { SplitDir } from "../state/layout";
import {
  buildNewSessionCommand,
  PRESET_SHORT,
  splitCommandString,
  type AgentPreset,
} from "../state/sessionCommand";

const MONTHS_ES = [
  "ene",
  "feb",
  "mar",
  "abr",
  "may",
  "jun",
  "jul",
  "ago",
  "sep",
  "oct",
  "nov",
  "dic",
];

function formatConvDateShort(epochSec: number): string {
  if (!epochSec) return "";
  const d = new Date(epochSec * 1000);
  return `${d.getDate()} ${MONTHS_ES[d.getMonth()]}`;
}

function formatConvDateFull(epochSec: number): string {
  if (!epochSec) return "";
  const d = new Date(epochSec * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${d.getDate()} ${MONTHS_ES[d.getMonth()]} ${hh}:${mm}`;
}

export interface NewSessionFormResult {
  projectId: string | null;
  accountId: string | null;
  command: string[];
  title: string | null;
  cwd: string;
  openInSplit: boolean;
  splitDir: SplitDir;
}

interface NewSessionDialogProps {
  open: boolean;
  projects: ProjectInfo[];
  accounts: AccountInfo[];
  quotasByAccountId: Map<string, Quota | null>;
  defaultProjectId: string | null;
  defaultAccountId?: string | null;
  selectedProjectIdOverride: string | null;
  onConsumeProjectIdOverride: () => void;
  onRequestAddProject: () => void;
  suppressed?: boolean;
  canSplit: boolean;
  defaultOpenInSplit: boolean;
  defaultSplitDir: SplitDir;
  onClose: () => void;
  onSubmit: (data: NewSessionFormResult) => Promise<void>;
}

function shortenPath(path: string): string {
  const homeMatch = path.match(/^\/home\/[^/]+(.*)$/);
  return homeMatch ? `~${homeMatch[1]}` : path;
}

export function NewSessionDialog({
  open,
  projects,
  accounts,
  quotasByAccountId,
  defaultProjectId,
  defaultAccountId = null,
  selectedProjectIdOverride,
  onConsumeProjectIdOverride,
  onRequestAddProject,
  suppressed = false,
  canSplit,
  defaultOpenInSplit,
  defaultSplitDir,
  onClose,
  onSubmit,
}: NewSessionDialogProps) {
  const [projectId, setProjectId] = useState<string>("");
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [agent, setAgent] = useState<AgentPreset>("claude");
  const [accountId, setAccountId] = useState<string | null>(null);
  const [accountDropdownOpen, setAccountDropdownOpen] = useState(false);
  const [customCmd, setCustomCmd] = useState("");
  const [customCwd, setCustomCwd] = useState("");
  const [conversations, setConversations] = useState<ClaudeConversation[]>([]);
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [convDropdownOpen, setConvDropdownOpen] = useState(false);
  const [loadingConvs, setLoadingConvs] = useState(false);
  const [title, setTitle] = useState("");
  const [titleTouched, setTitleTouched] = useState(false);
  const [openInSplit, setOpenInSplit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const cardRef = useRef<HTMLFormElement | null>(null);
  const selectWrapRef = useRef<HTMLDivElement | null>(null);
  const accountSelectWrapRef = useRef<HTMLDivElement | null>(null);
  const convSelectWrapRef = useRef<HTMLDivElement | null>(null);
  const wasOpenRef = useRef(false);

  useEffect(() => {
    if (!open) {
      wasOpenRef.current = false;
      return;
    }
    if (wasOpenRef.current) return;
    wasOpenRef.current = true;

    const initialProject =
      defaultProjectId && projects.some((p) => p.id === defaultProjectId)
        ? defaultProjectId
        : projects[0]?.id ?? "";
    const claudeAccounts = filterAccountsByAgent(accounts, "claude");
    const initialAccount =
      defaultAccountId && claudeAccounts.some((a) => a.id === defaultAccountId)
        ? defaultAccountId
        : claudeAccounts[0]?.id ?? null;

    setProjectId(initialProject);
    setDropdownOpen(false);
    setAccountDropdownOpen(false);
    setConvDropdownOpen(false);
    setAgent("claude");
    setAccountId(initialAccount);
    setCustomCmd("");
    setCustomCwd("");
    setConversations([]);
    setConversationId(null);
    setTitleTouched(false);
    setOpenInSplit(canSplit && defaultOpenInSplit);
    setError(null);
    setSubmitting(false);

    const proj = projects.find((p) => p.id === initialProject);
    setTitle(proj ? `claude · ${proj.name}` : "claude");

    const frame = window.requestAnimationFrame(() => {
      cardRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    open,
    projects,
    accounts,
    defaultProjectId,
    defaultAccountId,
    canSplit,
    defaultOpenInSplit,
  ]);

  useEffect(() => {
    if (!open || !selectedProjectIdOverride) return;
    if (projects.some((p) => p.id === selectedProjectIdOverride)) {
      setProjectId(selectedProjectIdOverride);
      setConversationId(null);
      onConsumeProjectIdOverride();
    }
  }, [open, selectedProjectIdOverride, projects, onConsumeProjectIdOverride]);

  const selectedProject = projects.find((p) => p.id === projectId) ?? null;
  const effectiveCwd = selectedProject ? selectedProject.path : customCwd.trim();
  const agentAccounts = filterAccountsByAgent(accounts, agent);
  const selectedAccount =
    agentAccounts.find((a) => a.id === accountId) ?? null;

  useEffect(() => {
    if (!open || agent !== "claude") return;
    setConversationId(null);
    if (!effectiveCwd) {
      setConversations([]);
      setLoadingConvs(false);
      return;
    }

    let cancelled = false;
    setLoadingConvs(true);
    void api
      .claudeConversations(effectiveCwd, accountId)
      .then((list) => {
        if (!cancelled) {
          setConversations(list);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setConversations([]);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingConvs(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [open, agent, effectiveCwd, accountId]);

  useEffect(() => {
    if (!open || titleTouched) return;
    const proj = projects.find((p) => p.id === projectId);
    const prefix =
      agent === "custom"
        ? splitCommandString(customCmd)[0] || "cmd"
        : PRESET_SHORT[agent];
    setTitle(proj ? `${prefix} · ${proj.name}` : prefix);
  }, [open, projectId, agent, customCmd, projects, titleTouched]);

  useEffect(() => {
    if (!open || suppressed) return;
    const anyDropdownOpen = dropdownOpen || accountDropdownOpen || convDropdownOpen;
    const onKey = (e: KeyboardEvent) => {
      const action = resolveDialogKey(e.key, anyDropdownOpen);
      if (action === "close-dropdown") {
        e.preventDefault();
        e.stopPropagation();
        setDropdownOpen(false);
        setAccountDropdownOpen(false);
        setConvDropdownOpen(false);
      } else if (action === "close-dialog") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, suppressed, dropdownOpen, accountDropdownOpen, convDropdownOpen, onClose]);

  useEffect(() => {
    if (!dropdownOpen && !accountDropdownOpen && !convDropdownOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (dropdownOpen && selectWrapRef.current && !selectWrapRef.current.contains(target)) {
        setDropdownOpen(false);
      }
      if (
        accountDropdownOpen &&
        accountSelectWrapRef.current &&
        !accountSelectWrapRef.current.contains(target)
      ) {
        setAccountDropdownOpen(false);
      }
      if (
        convDropdownOpen &&
        convSelectWrapRef.current &&
        !convSelectWrapRef.current.contains(target)
      ) {
        setConvDropdownOpen(false);
      }
    };
    window.addEventListener("mousedown", onMouseDown);
    return () => window.removeEventListener("mousedown", onMouseDown);
  }, [dropdownOpen, accountDropdownOpen, convDropdownOpen]);

  if (!open) return null;

  const selectedConversation =
    conversations.find((c) => c.id === conversationId) ?? null;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);

    const command = buildNewSessionCommand(agent, customCmd, conversationId);
    if (command.length === 0) {
      setError(es.newSessionDialog.errorNoCommand);
      return;
    }

    const cwd = selectedProject ? selectedProject.path : customCwd.trim();
    if (!cwd) {
      setError(es.newSessionDialog.errorNoCwd);
      return;
    }

    setSubmitting(true);
    try {
      await onSubmit({
        projectId: selectedProject ? selectedProject.id : null,
        accountId: agent === "custom" ? null : selectedAccount?.id ?? null,
        command,
        title: title.trim() || null,
        cwd,
        openInSplit: canSplit && openInSplit,
        splitDir: defaultSplitDir,
      });
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      className="pas-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="new-session-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !suppressed) onClose();
      }}
    >
      <form
        ref={cardRef}
        tabIndex={-1}
        className="pas-dialog-card is-wide"
        style={{ outline: "none" }}
        onSubmit={handleSubmit}
      >
        <h4 id="new-session-title">{es.newSessionDialog.title}</h4>

        <div className="pas-field">
          <label id="ns-project-label">{es.newSessionDialog.projectLabel}</label>
          <div ref={selectWrapRef} className="pas-select-wrap">
            <button
              type="button"
              role="combobox"
              aria-labelledby="ns-project-label"
              aria-expanded={dropdownOpen}
              aria-haspopup="listbox"
              className="pas-select-trigger"
              onClick={() => {
                setAccountDropdownOpen(false);
                setConvDropdownOpen(false);
                setDropdownOpen((v) => !v);
              }}
            >
              <span className="pas-select-label">
                {selectedProject
                  ? `${selectedProject.name} · ${shortenPath(selectedProject.path)}`
                  : es.newSessionDialog.noProjectOption}
              </span>
              <ChevronDown size={14} className="pas-select-chevron" />
            </button>

            {dropdownOpen && (
              <div className="pas-select-menu" role="listbox">
                {projects.map((p) => {
                  const isSelected = p.id === projectId;
                  return (
                    <button
                      key={p.id}
                      type="button"
                      role="option"
                      aria-selected={isSelected}
                      className={`pas-select-option${isSelected ? " is-selected" : ""}`}
                      onClick={() => {
                        setProjectId(p.id);
                        setDropdownOpen(false);
                      }}
                    >
                      <span className="pas-select-option-text">
                        {p.name}{" "}
                        <span className="pas-select-option-path">
                          · {shortenPath(p.path)}
                        </span>
                      </span>
                      {isSelected && (
                        <Check size={13} style={{ color: "var(--acc)", flex: "none" }} />
                      )}
                    </button>
                  );
                })}
                <button
                  type="button"
                  role="option"
                  aria-selected={projectId === ""}
                  className={`pas-select-option${projectId === "" ? " is-selected" : ""}`}
                  onClick={() => {
                    setProjectId("");
                    setDropdownOpen(false);
                  }}
                >
                  <span className="pas-select-option-text">
                    {es.newSessionDialog.noProjectOption}
                  </span>
                  {projectId === "" && (
                    <Check size={13} style={{ color: "var(--acc)", flex: "none" }} />
                  )}
                </button>
                <div className="pas-menu-sep" />
                <button
                  type="button"
                  role="option"
                  aria-selected={false}
                  className="pas-select-option is-accent"
                  onClick={() => {
                    setDropdownOpen(false);
                    onRequestAddProject();
                  }}
                >
                  <FolderPlus size={13} style={{ flex: "none" }} />
                  <span className="pas-select-option-text">
                    {es.newSessionDialog.addProjectOption}
                  </span>
                </button>
              </div>
            )}
          </div>
        </div>

        {!selectedProject && (
          <div className="pas-field">
            <label htmlFor="ns-cwd">{es.newSessionDialog.cwdLabel}</label>
            <input
              id="ns-cwd"
              className="pas-input"
              type="text"
              value={customCwd}
              placeholder={es.newSessionDialog.cwdPlaceholder}
              onChange={(e) => setCustomCwd(e.target.value)}
            />
          </div>
        )}

        <div className="pas-field">
          <label>{es.newSessionDialog.agentLabel}</label>
          <div className="pas-seg" role="radiogroup">
            {(["claude", "codex", "agy", "custom"] as AgentPreset[]).map((preset) => (
              <button
                key={preset}
                type="button"
                role="radio"
                aria-checked={agent === preset}
                className={`pas-seg-btn${agent === preset ? " is-active" : ""}`}
                onClick={() => {
                  setAgent(preset);
                  const nextAccounts = filterAccountsByAgent(accounts, preset);
                  setAccountId(nextAccounts[0]?.id ?? null);
                  setAccountDropdownOpen(false);
                  setConvDropdownOpen(false);
                }}
              >
                {es.newSessionDialog.agents[preset]}
              </button>
            ))}
          </div>
        </div>

        {agent !== "custom" && (
          <div className="pas-field">
            <label id="ns-account-label">{es.newSessionDialog.accountLabel}</label>
            <div ref={accountSelectWrapRef} className="pas-select-wrap">
              <button
                type="button"
                role="combobox"
                aria-labelledby="ns-account-label"
                aria-expanded={accountDropdownOpen}
                aria-haspopup="listbox"
                className="pas-select-trigger"
                disabled={agentAccounts.length === 0}
                onClick={() => {
                  setDropdownOpen(false);
                  setConvDropdownOpen(false);
                  setAccountDropdownOpen((v) => !v);
                }}
              >
                <span className="pas-select-label">
                  {selectedAccount ? (
                    <>
                      <span>{selectedAccount.name}</span>
                      <span className="pas-select-inline-quota">
                        {formatQuotaInline(
                          quotasByAccountId.get(selectedAccount.id),
                          es.quota.noData,
                        )}
                      </span>
                    </>
                  ) : agent === "agy" ? (
                    es.newSessionDialog.agySystemAccount
                  ) : (
                    es.newSessionDialog.noAccountOption
                  )}
                </span>
                <ChevronDown size={14} className="pas-select-chevron" />
              </button>

              {accountDropdownOpen && agentAccounts.length > 0 && (
                <div className="pas-select-menu" role="listbox">
                  {agentAccounts.map((acc) => {
                    const isSelected = acc.id === accountId;
                    const quotaSummary = formatQuotaInline(
                      quotasByAccountId.get(acc.id),
                      es.quota.noData,
                    );
                    return (
                      <button
                        key={acc.id}
                        type="button"
                        role="option"
                        aria-selected={isSelected}
                        className={`pas-select-option${isSelected ? " is-selected" : ""}`}
                        onClick={() => {
                          setAccountId(acc.id);
                          setAccountDropdownOpen(false);
                        }}
                      >
                        <span className="pas-select-option-text">{acc.name}</span>
                        <span className="pas-select-option-meta">{quotaSummary}</span>
                        {isSelected && (
                          <Check size={13} style={{ color: "var(--acc)", flex: "none" }} />
                        )}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        )}

        {agent === "claude" && (
          <div className="pas-field">
            <label id="ns-conv-label">{es.newSessionDialog.conversationLabel}</label>
            <div ref={convSelectWrapRef} className="pas-select-wrap">
              <button
                type="button"
                role="combobox"
                aria-labelledby="ns-conv-label"
                aria-expanded={convDropdownOpen}
                aria-haspopup="listbox"
                className="pas-select-trigger"
                onClick={() => {
                  setDropdownOpen(false);
                  setAccountDropdownOpen(false);
                  setConvDropdownOpen((v) => !v);
                }}
              >
                <span className="pas-select-label">
                  {selectedConversation
                    ? `${selectedConversation.title}${
                        selectedConversation.modified_at
                          ? ` · ${formatConvDateShort(selectedConversation.modified_at)}`
                          : ""
                      }`
                    : loadingConvs
                      ? es.newSessionDialog.loadingConversations
                      : es.newSessionDialog.newConversation}
                </span>
                <ChevronDown size={14} className="pas-select-chevron" />
              </button>

              {convDropdownOpen && (
                <div className="pas-select-menu" role="listbox">
                  <button
                    type="button"
                    role="option"
                    aria-selected={conversationId === null}
                    className={`pas-select-option${conversationId === null ? " is-selected" : ""}`}
                    onClick={() => {
                      setConversationId(null);
                      setConvDropdownOpen(false);
                    }}
                  >
                    <span className="pas-select-option-text">
                      {es.newSessionDialog.newConversation}
                    </span>
                    {conversationId === null && (
                      <Check size={13} style={{ color: "var(--acc)", flex: "none" }} />
                    )}
                  </button>
                  {conversations.map((conv) => {
                    const isSelected = conv.id === conversationId;
                    return (
                      <button
                        key={conv.id}
                        type="button"
                        role="option"
                        aria-selected={isSelected}
                        className={`pas-select-option${isSelected ? " is-selected" : ""}`}
                        onClick={() => {
                          setConversationId(conv.id);
                          setConvDropdownOpen(false);
                        }}
                      >
                        <span className="pas-select-option-text">{conv.title}</span>
                        <span className="pas-select-option-meta">
                          {formatConvDateFull(conv.modified_at)}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        )}

        {agent === "custom" && (
          <div className="pas-field">
            <label htmlFor="ns-cmd">{es.newSessionDialog.commandLabel}</label>
            <input
              id="ns-cmd"
              className="pas-input"
              type="text"
              autoFocus
              value={customCmd}
              placeholder={es.newSessionDialog.commandPlaceholder}
              onChange={(e) => setCustomCmd(e.target.value)}
            />
          </div>
        )}

        <div className="pas-field">
          <label htmlFor="ns-title">{es.newSessionDialog.titleLabel}</label>
          <input
            id="ns-title"
            className="pas-input"
            type="text"
            value={title}
            placeholder={es.newSessionDialog.titlePlaceholder}
            onChange={(e) => {
              setTitleTouched(true);
              setTitle(e.target.value);
            }}
          />
        </div>

        {canSplit && (
          <label className="pas-check">
            <input
              type="checkbox"
              checked={openInSplit}
              onChange={(e) => setOpenInSplit(e.target.checked)}
            />
            <span>{es.newSessionDialog.openInSplit}</span>
          </label>
        )}

        {error && <p className="pas-field-error">{error}</p>}

        <div className="pas-dialog-actions">
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.newSessionDialog.cancel}
          </button>
          <button
            type="submit"
            className="pas-btn is-primary"
            disabled={submitting}
          >
            {submitting
              ? es.newSessionDialog.submitting
              : es.newSessionDialog.submit}
          </button>
        </div>
      </form>
    </div>
  );
}
