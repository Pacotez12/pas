import { useEffect, useRef, useState } from "react";
import { Code2, Copy, FolderOpen, Terminal } from "lucide-react";
import type { OpenTarget } from "../api/pas";
import { es } from "../i18n/es";

interface OpenFolderMenuProps {
  folderPath: string;
  onOpenPath: (path: string, target: OpenTarget) => void;
  onCopyPath: (path: string) => void;
}

export function OpenFolderMenu({
  folderPath,
  onOpenPath,
  onCopyPath,
}: OpenFolderMenuProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const btnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function toggleMenu(e: React.MouseEvent<HTMLButtonElement>) {
    e.stopPropagation();
    if (open) {
      setOpen(false);
      return;
    }
    const rect = e.currentTarget.getBoundingClientRect();
    const menuWidth = 250;
    const menuHeight = 148;
    const left = Math.max(8, Math.min(rect.right - menuWidth, window.innerWidth - menuWidth - 8));
    const top = Math.max(8, Math.min(rect.bottom + 4, window.innerHeight - menuHeight - 8));
    setPos({ left, top });
    setOpen(true);
  }

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        className={`pas-pane-btn${open ? " is-active" : ""}`}
        title={es.pane.openFolderTooltip}
        aria-label={es.pane.openFolderTooltip}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggleMenu}
      >
        <FolderOpen size={12} strokeWidth={2} />
      </button>

      {open && pos && (
        <div
          className="pas-menu pas-folder-menu"
          role="menu"
          style={{ left: pos.left, top: pos.top }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onClick={() => {
              setOpen(false);
              onOpenPath(folderPath, "editor");
            }}
          >
            <Code2 size={14} strokeWidth={2} className="pas-menu-ico" />
            <span>{es.contextMenu.openInEditor}</span>
            <kbd className="pas-menu-kbd">{es.contextMenu.openInEditorShortcut}</kbd>
          </button>
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onClick={() => {
              setOpen(false);
              onOpenPath(folderPath, "files");
            }}
          >
            <FolderOpen size={14} strokeWidth={2} className="pas-menu-ico" />
            <span>{es.contextMenu.openInFiles}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onClick={() => {
              setOpen(false);
              onOpenPath(folderPath, "terminal");
            }}
          >
            <Terminal size={14} strokeWidth={2} className="pas-menu-ico" />
            <span>{es.contextMenu.openInTerminal}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onClick={() => {
              setOpen(false);
              onCopyPath(folderPath);
            }}
          >
            <Copy size={14} strokeWidth={2} className="pas-menu-ico" />
            <span>{es.contextMenu.copyPath}</span>
          </button>
        </div>
      )}
    </>
  );
}
