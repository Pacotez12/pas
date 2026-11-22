import { Plus, X } from "lucide-react";
import type { SessionInfo, SessionState } from "../api/pas";
import { es } from "../i18n/es";
import type { TabLayout } from "../state/layout";

interface TabsProps {
  tabs: TabLayout[];
  activeTabId: string | null;
  sessionsById: Map<string, SessionInfo>;
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onNewSession: () => void;
}

export function statusDotClass(state: SessionState | undefined): string {
  if (!state) return "is-idle";
  if (state.state === "running") return "is-ok";
  if (state.state === "idle") return "is-idle";
  return state.code === 0 ? "is-idle" : "is-err";
}

export function Tabs({
  tabs,
  activeTabId,
  sessionsById,
  onSelectTab,
  onCloseTab,
  onNewSession,
}: TabsProps) {
  return (
    <div className="pas-tabs" role="tablist">
      {tabs.map((tab) => {
        const focused = sessionsById.get(tab.focusedSessionId);
        const isActive = tab.id === activeTabId;
        const label = focused?.title || tab.focusedSessionId;
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={isActive}
            tabIndex={isActive ? 0 : -1}
            className={`pas-tab${isActive ? " is-active" : ""}`}
            onClick={() => onSelectTab(tab.id)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onSelectTab(tab.id);
              }
            }}
          >
            <span className={`pas-dot ${statusDotClass(focused?.state)}`} />
            <span>{label}</span>
            <button
              type="button"
              className="pas-tab-close"
              title={es.tabs.closeTabTooltip}
              aria-label={es.tabs.closeTabTooltip}
              onClick={(e) => {
                e.stopPropagation();
                onCloseTab(tab.id);
              }}
            >
              <X size={11} strokeWidth={2.2} />
            </button>
          </div>
        );
      })}
      <button
        type="button"
        className="pas-tab-plus"
        title={es.tabs.newTabTooltip}
        aria-label={es.tabs.newTabTooltip}
        onClick={onNewSession}
      >
        <Plus size={14} strokeWidth={2.2} />
      </button>
    </div>
  );
}
