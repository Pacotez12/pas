import { useEffect, useMemo, useRef, useState } from "react";
import {
  Code2,
  Folder,
  FolderPlus,
  Plus,
  Search,
  Settings,
} from "lucide-react";
import type { ProjectInfo, SessionInfo, SessionState } from "../api/pas";
import { es } from "../i18n/es";
import {
  buildPaletteItems,
  filterPaletteItems,
  type PaletteAction,
  type PaletteItem,
  type PaletteSection,
} from "../state/palette";
import { statusDotClass } from "./Tabs";

interface CommandPaletteProps {
  open: boolean;
  sessions: SessionInfo[];
  projects: ProjectInfo[];
  hasActiveSession: boolean;
  onClose: () => void;
  onSelectSession: (session: SessionInfo) => void;
  onNewSessionForProject: (project: ProjectInfo) => void;
  onOpenPathInEditor: (path: string) => void;
  onRunAction: (actionId: string) => void;
}

function sessionStateLabel(state: SessionState): string {
  if (state.state === "running") return es.sessionState.running;
  if (state.state === "waiting") return es.sessionState.waiting;
  if (state.state === "idle") return es.sessionState.idle;
  if (state.code === 0) return es.sessionState.exitedOk;
  return es.sessionState.exitedError(state.code);
}

function sectionTitle(section: PaletteSection): string {
  switch (section) {
    case "sessions":
      return es.palette.sectionSessions;
    case "projects":
      return es.palette.sectionProjects;
    case "actions":
      return es.palette.sectionActions;
  }
}

function ActionIcon({ id }: { id: string }) {
  switch (id) {
    case "new-session":
      return <Plus size={14} strokeWidth={2} className="pas-pal-ico" />;
    case "add-project":
      return <FolderPlus size={14} strokeWidth={2} className="pas-pal-ico" />;
    case "accounts":
      return <Settings size={14} strokeWidth={2} className="pas-pal-ico" />;
    case "open-current-editor":
      return <Code2 size={14} strokeWidth={2} className="pas-pal-ico" />;
    default:
      return <Search size={14} strokeWidth={2} className="pas-pal-ico" />;
  }
}

export function CommandPalette({
  open,
  sessions,
  projects,
  hasActiveSession,
  onClose,
  onSelectSession,
  onNewSessionForProject,
  onOpenPathInEditor,
  onRunAction,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  const actions = useMemo<PaletteAction[]>(() => {
    const list: PaletteAction[] = [
      {
        id: "new-session",
        label: es.palette.actionNewSession,
        shortcut: es.sidebar.newSessionShortcut,
      },
      {
        id: "add-project",
        label: es.palette.actionAddProject,
      },
      {
        id: "accounts",
        label: es.palette.actionAccounts,
      },
    ];
    if (hasActiveSession) {
      list.push({
        id: "open-current-editor",
        label: es.palette.actionOpenCurrentInEditor,
        shortcut: es.contextMenu.openInEditorShortcut,
      });
    }
    return list;
  }, [hasActiveSession]);

  const allItems = useMemo(
    () => buildPaletteItems({ sessions, projects, actions }),
    [sessions, projects, actions],
  );

  const filtered = useMemo(
    () => filterPaletteItems(allItems, query),
    [allItems, query],
  );

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelectedIndex(0);
    // Focus immediately without animation delay.
    inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    setSelectedIndex((prev) => {
      if (filtered.length === 0) return 0;
      return Math.min(prev, filtered.length - 1);
    });
  }, [filtered.length]);

  useEffect(() => {
    if (!open) return;
    const container = listRef.current;
    if (!container) return;
    const activeEl = container.querySelector<HTMLElement>(
      `[data-palette-index="${selectedIndex}"]`,
    );
    activeEl?.scrollIntoView({ block: "nearest" });
  }, [open, selectedIndex]);

  if (!open) return null;

  function activateItem(item: PaletteItem, altKey: boolean) {
    onClose();
    if (item.section === "sessions") {
      if (altKey) {
        onOpenPathInEditor(item.session.worktree?.path ?? item.session.cwd);
      } else {
        onSelectSession(item.session);
      }
      return;
    }
    if (item.section === "projects") {
      if (altKey) {
        onOpenPathInEditor(item.project.path);
      } else {
        onNewSessionForProject(item.project);
      }
      return;
    }
    onRunAction(item.action.id);
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (filtered.length > 0) {
        setSelectedIndex((prev) => (prev + 1) % filtered.length);
      }
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (filtered.length > 0) {
        setSelectedIndex((prev) => (prev - 1 + filtered.length) % filtered.length);
      }
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const target = filtered[selectedIndex];
      if (target) {
        activateItem(target, e.altKey);
      }
    }
  }

  // Group filtered items preserving their flat indices for keyboard navigation.
  const sections: Array<{
    section: PaletteSection;
    entries: Array<{ item: PaletteItem; flatIndex: number }>;
  }> = [];
  for (let i = 0; i < filtered.length; i += 1) {
    const item = filtered[i];
    const last = sections[sections.length - 1];
    if (last && last.section === item.section) {
      last.entries.push({ item, flatIndex: i });
    } else {
      sections.push({
        section: item.section,
        entries: [{ item, flatIndex: i }],
      });
    }
  }

  return (
    <div
      className="pas-pal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={es.app.searchTooltip}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onKeyDown={handleKeyDown}
    >
      <div className="pas-pal">
        <div className="pas-pal-q-wrap">
          <Search size={15} strokeWidth={2} className="pas-pal-search-ico" />
          <input
            ref={inputRef}
            type="text"
            className="pas-pal-q"
            value={query}
            placeholder={es.palette.placeholder}
            aria-label={es.palette.placeholder}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedIndex(0);
            }}
          />
        </div>

        <div ref={listRef} className="pas-pal-list" role="listbox">
          {filtered.length === 0 ? (
            <div className="pas-pal-empty">{es.palette.empty}</div>
          ) : (
            sections.map((group) => (
              <div key={group.section}>
                <div className="pas-pal-sec">{sectionTitle(group.section)}</div>
                {group.entries.map(({ item, flatIndex }) => {
                  const isActive = flatIndex === selectedIndex;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      role="option"
                      aria-selected={isActive}
                      data-palette-index={flatIndex}
                      data-palette-id={item.id}
                      className={`pas-pal-it${isActive ? " is-active" : ""}`}
                      onMouseEnter={() => setSelectedIndex(flatIndex)}
                      onClick={(e) => activateItem(item, e.altKey)}
                    >
                      {item.section === "sessions" ? (
                        <>
                          <span
                            className={`pas-dot ${statusDotClass(item.session.state)}`}
                          />
                          <span className="pas-pal-label">{item.label}</span>
                          <small className="pas-pal-meta">
                            {item.session.mode === "chat"
                              ? es.palette.sessionModeChat
                              : es.palette.sessionModeTerminal}{" "}
                            · {sessionStateLabel(item.session.state)}
                          </small>
                        </>
                      ) : item.section === "projects" ? (
                        <>
                          <Folder size={14} strokeWidth={2} className="pas-pal-ico" />
                          <span className="pas-pal-label">{item.label}</span>
                          <small className="pas-pal-meta">
                            {es.palette.projectHint}
                          </small>
                        </>
                      ) : (
                        <>
                          <ActionIcon id={item.action.id} />
                          <span className="pas-pal-label">{item.label}</span>
                          {item.action.shortcut && (
                            <small className="pas-pal-meta pas-pal-kbd">
                              {item.action.shortcut}
                            </small>
                          )}
                        </>
                      )}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
