import { useEffect, useState, type FormEvent } from "react";
import { es } from "../i18n/es";

interface AddProjectDialogProps {
  open: boolean;
  onClose: () => void;
  onSubmit: (path: string, name?: string) => Promise<void>;
}

export function AddProjectDialog({
  open,
  onClose,
  onSubmit,
}: AddProjectDialogProps) {
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setPath("");
    setName("");
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
  }, [open, onClose]);

  if (!open) return null;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const trimmed = path.trim();
    if (!trimmed) {
      setError(es.addProjectDialog.errorEmptyPath);
      return;
    }
    setError(null);
    setSubmitting(true);
    try {
      await onSubmit(trimmed, name.trim() || undefined);
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
      aria-labelledby="add-project-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <form className="pas-dialog-card" onSubmit={handleSubmit}>
        <h4 id="add-project-title">{es.addProjectDialog.title}</h4>
        <div className="pas-field">
          <label htmlFor="ap-path">{es.addProjectDialog.pathLabel}</label>
          <input
            id="ap-path"
            className="pas-input"
            type="text"
            autoFocus
            value={path}
            placeholder={es.addProjectDialog.pathPlaceholder}
            onChange={(e) => setPath(e.target.value)}
          />
        </div>
        <div className="pas-field">
          <label htmlFor="ap-name">{es.addProjectDialog.nameLabel}</label>
          <input
            id="ap-name"
            className="pas-input"
            type="text"
            value={name}
            placeholder={es.addProjectDialog.namePlaceholder}
            onChange={(e) => setName(e.target.value)}
          />
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
            {submitting
              ? es.addProjectDialog.submitting
              : es.addProjectDialog.submit}
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
