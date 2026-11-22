import { useEffect, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Folder,
  Plus,
} from "lucide-react";
import type {
  AccountInfo,
  GroupInfo,
  ProjectInfo,
  SessionInfo,
  SessionState,
} from "../api/pas";
import { es } from "../i18n/es";
import { statusDotClass } from "./Tabs";

interface SidebarProps {
  groups: GroupInfo[];
  projects: ProjectInfo[];
  sessions: SessionInfo[];
  accountsById: Map<string, AccountInfo>;
  activeSessionId: string | null;
  onNewSession: () => void;
  onSelectSession: (sessionId: string) => void;
  onAddProject: () => void;
  onRenameProject: (project: ProjectInfo) => void;
  onMoveProject: (projectId: string, groupId: string) => void;
  onCreateGroupAndMove: (project: ProjectInfo) => void;
  onUnpinProject: (projectId: string) => void;
  onRemoveProject: (project: ProjectInfo) => void;
  onRenameGroup: (group: GroupInfo) => void;
}

interface RepoContextState {
  project: ProjectInfo;
  x: number;
  y: number;
  showMoveSubmenu: boolean;
}

interface GroupContextState {
  group: GroupInfo;
  x: number;
  y: number;
}

function sessionStateLabel(state: SessionState): string {
  if (state.state === "running") return es.sessionState.running;
  if (state.state === "waiting") return es.sessionState.waiting;
  if (state.state === "idle") return es.sessionState.idle;
  if (state.code === 0) return es.sessionState.exitedOk;
  return es.sessionState.exitedError(state.code);
}

function folderBasename(path: string): string {
  const clean = path.replace(/\/+$/, "");
  const parts = clean.split("/");
  return parts[parts.length - 1] || clean;
}

function clampMenuPosition(x: number, y: number, width: number, height: number) {
  const maxX = Math.max(8, window.innerWidth - width - 8);
  const maxY = Math.max(8, window.innerHeight - height - 8);
  return {
    left: Math.min(x, maxX),
    top: Math.min(y, maxY),
  };
}

export function Sidebar({
  groups,
  projects,
  sessions,
  accountsById,
  activeSessionId,
  onNewSession,
  onSelectSession,
  onAddProject,
  onRenameProject,
  onMoveProject,
  onCreateGroupAndMove,
  onUnpinProject,
  onRemoveProject,
  onRenameGroup,
}: SidebarProps) {
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [collapsedRepos, setCollapsedRepos] = useState<Set<string>>(new Set());
  const [dragOverGroupId, setDragOverGroupId] = useState<string | null>(null);
  const [repoMenu, setRepoMenu] = useState<RepoContextState | null>(null);
  const [groupMenu, setGroupMenu] = useState<GroupContextState | null>(null);

  useEffect(() => {
    if (!repoMenu && !groupMenu) return;
    const close = () => {
      setRepoMenu(null);
      setGroupMenu(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [repoMenu, groupMenu]);

  const projectsByGroup = new Map<string, ProjectInfo[]>();
  for (const project of projects) {
    const list = projectsByGroup.get(project.group_id) ?? [];
    list.push(project);
    projectsByGroup.set(project.group_id, list);
  }

  const sessionsByProject = new Map<string, SessionInfo[]>();
  const ungroupedSessions: SessionInfo[] = [];
  const knownProjectIds = new Set(projects.map((p) => p.id));

  for (const session of sessions) {
    if (session.project_id && knownProjectIds.has(session.project_id)) {
      const list = sessionsByProject.get(session.project_id) ?? [];
      list.push(session);
      sessionsByProject.set(session.project_id, list);
    } else {
      ungroupedSessions.push(session);
    }
  }

  function toggleGroup(id: string) {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleRepo(id: string) {
    setCollapsedRepos((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function renderSession(session: SessionInfo, indentClass: "is-sub" | "is-sub0" | "") {
    const isActive = session.id === activeSessionId;
    const account = session.account_id
      ? accountsById.get(session.account_id)
      : undefined;
    const stateLabel = sessionStateLabel(session.state);
    const metaParts: string[] = [];
    if (session.mode === "chat") {
      metaParts.push(es.chat.modeTag);
    }
    if (account) {
      metaParts.push(account.name);
    }
    metaParts.push(stateLabel);
    const metaText = metaParts.join(" · ");
    return (
      <button
        key={session.id}
        type="button"
        className={`pas-ses${indentClass ? ` ${indentClass}` : ""}${isActive ? " is-active" : ""}`}
        data-session-id={session.id}
        onClick={() => onSelectSession(session.id)}
      >
        <span className={`pas-dot ${statusDotClass(session.state)}`} />
        <span className="pas-ses-text">
          <span className="pas-ses-title" style={{ display: "block" }}>
            {session.title}
          </span>
          <small className="pas-ses-meta">{metaText}</small>
        </span>
      </button>
    );
  }

  const repoMenuPos = repoMenu
    ? clampMenuPosition(repoMenu.x, repoMenu.y, 230, 170)
    : null;
  const submenuPos =
    repoMenu && repoMenuPos && repoMenu.showMoveSubmenu
      ? clampMenuPosition(
          repoMenuPos.left + 234,
          repoMenuPos.top + 28,
          200,
          Math.min(320, groups.length * 32 + 50),
        )
      : null;

  const groupMenuPos = groupMenu
    ? clampMenuPosition(groupMenu.x, groupMenu.y, 200, 60)
    : null;

  return (
    <aside className="pas-sidebar">
      <button type="button" className="pas-new-btn" onClick={onNewSession}>
        <Plus size={14} strokeWidth={2.4} />
        <span>{es.sidebar.newSession}</span>
        <kbd>{es.sidebar.newSessionShortcut}</kbd>
      </button>

      <div className="pas-sidebar-scroll">
        {groups.length === 0 && ungroupedSessions.length === 0 && (
          <div className="pas-sidebar-empty">
            <p>{es.sidebar.emptyProjects}</p>
            <span>{es.sidebar.emptyProjectsHint}</span>
          </div>
        )}

        {groups.map((group) => {
          const groupProjects = projectsByGroup.get(group.id) ?? [];
          if (groupProjects.length === 0) return null;

          const isGroupClosed = collapsedGroups.has(group.id);
          const singleProject = groupProjects.length === 1 ? groupProjects[0] : null;
          const groupSessions = groupProjects.flatMap(
            (p) => sessionsByProject.get(p.id) ?? [],
          );
          const isDropTarget = dragOverGroupId === group.id;

          return (
            <div
              key={group.id}
              className={`pas-group${isDropTarget ? " is-drop-target" : ""}`}
              onDragOver={(e) => {
                if (e.dataTransfer.types.includes("text/pas-project-id")) {
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  if (dragOverGroupId !== group.id) {
                    setDragOverGroupId(group.id);
                  }
                }
              }}
              onDragLeave={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
                  setDragOverGroupId((cur) => (cur === group.id ? null : cur));
                }
              }}
              onDrop={(e) => {
                e.preventDefault();
                setDragOverGroupId(null);
                const draggedProjectId = e.dataTransfer.getData("text/pas-project-id");
                if (draggedProjectId) {
                  const dragged = projects.find((p) => p.id === draggedProjectId);
                  if (dragged && dragged.group_id !== group.id) {
                    onMoveProject(draggedProjectId, group.id);
                  }
                }
              }}
            >
              <button
                type="button"
                className={`pas-fam${isGroupClosed ? " is-closed" : ""}`}
                draggable={singleProject !== null}
                onDragStart={(e) => {
                  if (!singleProject) return;
                  e.dataTransfer.setData("text/pas-project-id", singleProject.id);
                  e.dataTransfer.effectAllowed = "move";
                }}
                onClick={() => toggleGroup(group.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (singleProject) {
                    setGroupMenu(null);
                    setRepoMenu({
                      project: singleProject,
                      x: e.clientX,
                      y: e.clientY,
                      showMoveSubmenu: false,
                    });
                  } else {
                    setRepoMenu(null);
                    setGroupMenu({ group, x: e.clientX, y: e.clientY });
                  }
                }}
              >
                {isGroupClosed ? (
                  <ChevronRight size={11} strokeWidth={2.5} />
                ) : (
                  <ChevronDown size={11} strokeWidth={2.5} />
                )}
                <span className="pas-fam-name">{group.name}</span>
                <span className="pas-fam-cnt">
                  {es.sidebar.repoCount(groupProjects.length)}
                </span>
                {isGroupClosed && groupSessions.length > 0 && (
                  <span className="pas-mini-dots">
                    {groupSessions.map((s) => (
                      <span
                        key={s.id}
                        className={`pas-dot ${statusDotClass(s.state)}`}
                      />
                    ))}
                  </span>
                )}
              </button>

              {!isGroupClosed && singleProject && (
                <div>
                  {(sessionsByProject.get(singleProject.id) ?? []).map((s) =>
                    renderSession(s, "is-sub0"),
                  )}
                </div>
              )}

              {!isGroupClosed &&
                !singleProject &&
                groupProjects.map((project) => {
                  const repoSessions = sessionsByProject.get(project.id) ?? [];
                  const isRepoClosed = collapsedRepos.has(project.id);
                  const base = folderBasename(project.path);
                  const showBase = base.toLowerCase() !== project.name.toLowerCase();
                  const isContextOpen = repoMenu?.project.id === project.id;

                  return (
                    <div key={project.id}>
                      <button
                        type="button"
                        draggable
                        className={`pas-rep${isContextOpen ? " is-context-open" : ""}`}
                        onDragStart={(e) => {
                          e.dataTransfer.setData("text/pas-project-id", project.id);
                          e.dataTransfer.effectAllowed = "move";
                        }}
                        onClick={() => toggleRepo(project.id)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setGroupMenu(null);
                          setRepoMenu({
                            project,
                            x: e.clientX,
                            y: e.clientY,
                            showMoveSubmenu: false,
                          });
                        }}
                      >
                        {isRepoClosed ? (
                          <ChevronRight size={10} strokeWidth={2.6} />
                        ) : (
                          <ChevronDown size={10} strokeWidth={2.6} />
                        )}
                        <span className="pas-rep-name">{project.name}</span>
                        {showBase && <small>{base}</small>}
                        {isRepoClosed && repoSessions.length > 0 && (
                          <span className="pas-mini-dots">
                            {repoSessions.map((s) => (
                              <span
                                key={s.id}
                                className={`pas-dot ${statusDotClass(s.state)}`}
                              />
                            ))}
                          </span>
                        )}
                      </button>

                      {!isRepoClosed &&
                        repoSessions.map((s) => renderSession(s, "is-sub"))}
                    </div>
                  );
                })}
            </div>
          );
        })}

        {ungroupedSessions.length > 0 && (
          <div className="pas-group">
            <div className="pas-fam is-closed">
              <span className="pas-fam-name">{es.sidebar.ungroupedSessions}</span>
            </div>
            {ungroupedSessions.map((s) => renderSession(s, ""))}
          </div>
        )}
      </div>

      <button
        type="button"
        className="pas-add-project"
        onClick={onAddProject}
      >
        <Folder size={14} strokeWidth={2} />
        <span>{es.sidebar.addProject}</span>
      </button>

      {repoMenu && repoMenuPos && (
        <div
          className="pas-menu"
          role="menu"
          style={{ left: repoMenuPos.left, top: repoMenuPos.top }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onMouseEnter={() =>
              setRepoMenu((cur) => (cur ? { ...cur, showMoveSubmenu: false } : null))
            }
            onClick={() => {
              const target = repoMenu.project;
              setRepoMenu(null);
              onRenameProject(target);
            }}
          >
            {es.contextMenu.rename}
          </button>

          <button
            type="button"
            role="menuitem"
            className={`pas-menu-item${repoMenu.showMoveSubmenu ? " is-active" : ""}`}
            onMouseEnter={() =>
              setRepoMenu((cur) => (cur ? { ...cur, showMoveSubmenu: true } : null))
            }
            onClick={() =>
              setRepoMenu((cur) =>
                cur ? { ...cur, showMoveSubmenu: !cur.showMoveSubmenu } : null,
              )
            }
          >
            <span>{es.contextMenu.moveToGroup}</span>
            <ChevronRight size={13} style={{ marginLeft: "auto", color: "var(--mu)" }} />
          </button>

          <button
            type="button"
            role="menuitem"
            className={`pas-menu-item${!repoMenu.project.group_pinned ? " is-muted" : ""}`}
            disabled={!repoMenu.project.group_pinned}
            onMouseEnter={() =>
              setRepoMenu((cur) => (cur ? { ...cur, showMoveSubmenu: false } : null))
            }
            onClick={() => {
              const id = repoMenu.project.id;
              setRepoMenu(null);
              onUnpinProject(id);
            }}
          >
            {es.contextMenu.returnToAutoGroup}
          </button>

          <div className="pas-menu-sep" />

          <button
            type="button"
            role="menuitem"
            className="pas-menu-item is-danger"
            onMouseEnter={() =>
              setRepoMenu((cur) => (cur ? { ...cur, showMoveSubmenu: false } : null))
            }
            onClick={() => {
              const target = repoMenu.project;
              setRepoMenu(null);
              onRemoveProject(target);
            }}
          >
            {es.contextMenu.removeFromPas}
          </button>
        </div>
      )}

      {repoMenu && submenuPos && (
        <div
          className="pas-menu pas-submenu"
          role="menu"
          style={{ left: submenuPos.left, top: submenuPos.top }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {groups.map((g) => {
            const isCurrent = g.id === repoMenu.project.group_id;
            return (
              <button
                key={g.id}
                type="button"
                role="menuitem"
                className={`pas-menu-item${isCurrent ? " is-active" : ""}`}
                onClick={() => {
                  const pid = repoMenu.project.id;
                  setRepoMenu(null);
                  if (!isCurrent) {
                    onMoveProject(pid, g.id);
                  }
                }}
              >
                {g.name}
              </button>
            );
          })}
          <div className="pas-menu-sep" />
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item is-accent"
            onClick={() => {
              const target = repoMenu.project;
              setRepoMenu(null);
              onCreateGroupAndMove(target);
            }}
          >
            {es.contextMenu.newGroup}
          </button>
        </div>
      )}

      {groupMenu && groupMenuPos && (
        <div
          className="pas-menu"
          role="menu"
          style={{ left: groupMenuPos.left, top: groupMenuPos.top }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onClick={() => {
              const target = groupMenu.group;
              setGroupMenu(null);
              onRenameGroup(target);
            }}
          >
            {es.contextMenu.rename}
          </button>
        </div>
      )}
    </aside>
  );
}
