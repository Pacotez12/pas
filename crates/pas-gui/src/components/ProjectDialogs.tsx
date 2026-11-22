import { useEffect, useState, type FormEvent } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Check, FolderOpen } from "lucide-react";
import { api, type DiscoveredProject } from "../api/pas";
import { es } from "../i18n/es";

function shortenPath(path: string): string {
  const match = path.match(/^\/home\/[^/]+(.*)$/);
  return match ? `~${match[1]}` : path;
}

interface AddProjectDialogProps {
  open: boolean;
  onClose: () => void;
  onSubmit: (paths: string[]) => Promise<void>;
}

export function AddProjectDialog({
  open,
  onClose,
  onSubmit,
}: AddProjectDialogProps) {
  const [path, setPath] = useState("");
  const [discovered, setDiscovered] = useState<DiscoveredProject[]>([]);
  const [loadingDiscovered, setLoadingDiscovered] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(() => new Set());
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setPath("");
    setSelectedPaths(new Set());
    setError(null);
    setSubmitting(false);

    let cancelled = false;
    setLoadingDiscovered(true);
    void api
      .discoverProjects()
      .then((list) => {
        if (!cancelled) {
          setDiscovered(list);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setDiscovered([]);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingDiscovered(false);
        }
      });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      cancelled = true;
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, onClose]);

  if (!open) return null;

  function toggleSuggested(p: string) {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(p)) next.delete(p);
      else next.add(p);
      return next;
    });
  }

  async function handleChooseFolder() {
    try {
      const chosen = await openDialog({
        directory: true,
        multiple: false,
      });
      if (typeof chosen === "string" && chosen.trim()) {
        setPath(chosen);
        setError(null);
      }
    } catch (err) {
      setError(String(err));
    }
  }

  const trimmedPath = path.trim();
  const combinedPaths = Array.from(
    new Set([
      ...(trimmedPath ? [trimmedPath] : []),
      ...Array.from(selectedPaths),
    ]),
  );

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (combinedPaths.length === 0) {
      setError(es.addProjectDialog.errorEmptyPath);
      return;
    }
    setError(null);
    setSubmitting(true);
    try {
      await onSubmit(combinedPaths);
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  const submitLabel = submitting
    ? es.addProjectDialog.submitting
    : combinedPaths.length > 0
      ? es.addProjectDialog.submitCount(combinedPaths.length)
      : es.addProjectDialog.submit;

  return (
    <div
      className="pas-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="add-project-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <form className="pas-dialog-card is-project-add" onSubmit={handleSubmit}>
        <h4 id="add-project-title">{es.addProjectDialog.title}</h4>

        <div className="pas-field">
          <label htmlFor="ap-path">{es.addProjectDialog.pathLabel}</label>
          <div className="pas-path-row">
            <input
              id="ap-path"
              className="pas-input"
              type="text"
              autoFocus
              value={path}
              placeholder={es.addProjectDialog.pathPlaceholder}
              onChange={(e) => setPath(e.target.value)}
            />
            <button
              type="button"
              className="pas-btn pas-btn-inline"
              onClick={() => void handleChooseFolder()}
            >
              <FolderOpen size={13} strokeWidth={2} />
              <span>{es.addProjectDialog.chooseFolder}</span>
            </button>
          </div>
        </div>

        <div className="pas-field">
          <label>{es.addProjectDialog.discoveredLabel}</label>
          {loadingDiscovered ? (
            <div className="pas-sug-empty">
              {es.addProjectDialog.discoveredLoading}
            </div>
          ) : discovered.length === 0 ? (
            <div className="pas-sug-empty">
              {es.addProjectDialog.discoveredEmpty}
            </div>
          ) : (
            <div className="pas-sug" role="listbox" aria-multiselectable="true">
              {discovered.map((item) => {
                const isSelected = selectedPaths.has(item.path);
                return (
                  <button
                    key={item.path}
                    type="button"
                    role="option"
                    aria-selected={isSelected}
                    data-discovered-path={item.path}
                    className={`pas-sug-row${isSelected ? " is-selected" : ""}`}
                    onClick={() => toggleSuggested(item.path)}
                  >
                    <span
                      className={`pas-sug-box${isSelected ? " is-checked" : ""}`}
                    >
                      {isSelected && <Check size={10} strokeWidth={3} />}
                    </span>
                    <span className="pas-sug-name">{item.name}</span>
                    <span className="pas-sug-path" title={item.path}>
                      {shortenPath(item.path)}
                    </span>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        {error && <p className="pas-field-error">{error}</p>}

        <div className="pas-dialog-actions">
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.addProjectDialog.cancel}
          </button>
          <button
            type="submit"
            className="pas-btn is-primary"
            disabled={submitting}
          >
            {submitLabel}
          </button>
        </div>
      </form>
    </div>
  );
}

interface TextPromptDialogProps {
  open: boolean;
  title: string;
  initialValue: string;
  submitLabel: string;
  onClose: () => void;
  onSubmit: (value: string) => Promise<void>;
}

export function TextPromptDialog({
  open,
  title,
  initialValue,
  submitLabel,
  onClose,
  onSubmit,
}: TextPromptDialogProps) {
  const [value, setValue] = useState(initialValue);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setValue(initialValue);
    setError(null);
    setSubmitting(false);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, initialValue, onClose]);

  if (!open) return null;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const trimmed = value.trim();
    if (!trimmed) {
      setError(es.renameDialog.errorEmptyName);
      return;
    }
    setError(null);
    setSubmitting(true);
    try {
      await onSubmit(trimmed);
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
      aria-labelledby="text-prompt-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <form className="pas-dialog-card" onSubmit={handleSubmit}>
        <h4 id="text-prompt-title">{title}</h4>
        <div className="pas-field">
          <label htmlFor="tp-name">{es.renameDialog.nameLabel}</label>
          <input
            id="tp-name"
            className="pas-input"
            type="text"
            autoFocus
            value={value}
            placeholder={es.renameDialog.namePlaceholder}
            onChange={(e) => setValue(e.target.value)}
          />
        </div>
        {error && <p className="pas-field-error">{error}</p>}
        <div className="pas-dialog-actions">
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.renameDialog.cancel}
          </button>
          <button
            type="submit"
            className="pas-btn is-primary"
            disabled={submitting}
          >
            {submitLabel}
          </button>
        </div>
      </form>
    </div>
  );
}
