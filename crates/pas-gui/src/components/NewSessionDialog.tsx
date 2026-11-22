import { useEffect, useRef, useState, type FormEvent } from "react";
import { Check, ChevronDown } from "lucide-react";
import type { ProjectInfo } from "../api/pas";
import { es } from "../i18n/es";
import { resolveDialogKey } from "../state/dialog";
import type { SplitDir } from "../state/layout";

type AgentPreset = "claude" | "codex" | "agy" | "custom";

const PRESET_COMMANDS: Record<Exclude<AgentPreset, "custom">, string[]> = {
  claude: ["claude"],
  codex: ["codex"],
  agy: ["agy"],
};

const PRESET_SHORT: Record<Exclude<AgentPreset, "custom">, string> = {
  claude: "claude",
  codex: "codex",
  agy: "agy",
};

export interface NewSessionFormResult {
  projectId: string | null;
  command: string[];
  title: string | null;
  cwd: string;
  openInSplit: boolean;
  splitDir: SplitDir;
}

interface NewSessionDialogProps {
  open: boolean;
  projects: ProjectInfo[];
  defaultProjectId: string | null;
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

function splitCommandString(raw: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  for (let i = 0; i < raw.length; i += 1) {
    const ch = raw[i];
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else {
        current += ch;
      }
    } else if (ch === "'" || ch === '"') {
      quote = ch;
    } else if (/\s/.test(ch)) {
      if (current) {
        parts.push(current);
        current = "";
      }
    } else {
      current += ch;
    }
  }
  if (current) parts.push(current);
  return parts;
}

export function NewSessionDialog({
  open,
  projects,
  defaultProjectId,
  canSplit,
  defaultOpenInSplit,
  defaultSplitDir,
  onClose,
  onSubmit,
}: NewSessionDialogProps) {
  const [projectId, setProjectId] = useState<string>("");
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [agent, setAgent] = useState<AgentPreset>("claude");
  const [customCmd, setCustomCmd] = useState("");
  const [customCwd, setCustomCwd] = useState("");
  const [title, setTitle] = useState("");
  const [titleTouched, setTitleTouched] = useState(false);
  const [openInSplit, setOpenInSplit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const cardRef = useRef<HTMLFormElement | null>(null);
  const selectWrapRef = useRef<HTMLDivElement | null>(null);
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
    setProjectId(initialProject);
    setDropdownOpen(false);
    setAgent("claude");
    setCustomCmd("");
    setCustomCwd("");
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
  }, [open, projects, defaultProjectId, canSplit, defaultOpenInSplit]);

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
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      const action = resolveDialogKey(e.key, dropdownOpen);
      if (action === "close-dropdown") {
        e.preventDefault();
        e.stopPropagation();
        setDropdownOpen(false);
      } else if (action === "close-dialog") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, dropdownOpen, onClose]);

  useEffect(() => {
    if (!dropdownOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (
        selectWrapRef.current &&
        !selectWrapRef.current.contains(e.target as Node)
      ) {
        setDropdownOpen(false);
      }
    };
    window.addEventListener("mousedown", onMouseDown);
    return () => window.removeEventListener("mousedown", onMouseDown);
  }, [dropdownOpen]);

  if (!open) return null;

  const selectedProject = projects.find((p) => p.id === projectId) ?? null;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);

    const command =
      agent === "custom"
        ? splitCommandString(customCmd.trim())
        : PRESET_COMMANDS[agent];
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
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <form
        ref={cardRef}
        tabIndex={-1}
        className="pas-dialog-card"
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
              onClick={() => setDropdownOpen((v) => !v)}
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
                onClick={() => setAgent(preset)}
              >
                {es.newSessionDialog.agents[preset]}
              </button>
            ))}
          </div>
        </div>

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
