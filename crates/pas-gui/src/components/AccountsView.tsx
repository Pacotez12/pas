import { useEffect, useState, type FormEvent } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Check, FolderOpen, MoreHorizontal, Plus, Search } from "lucide-react";
import {
  api,
  type AccountInfo,
  type AgentKind,
  type DiscoveredAccount,
} from "../api/pas";
import { es } from "../i18n/es";
import { hooksTargetInfo, quotaPillLabel } from "../state/accounts";
import { TextPromptDialog } from "./ProjectDialogs";

const VIRTUAL_AGY_ID = "__agy_default__";

interface AccountsViewProps {
  accounts: AccountInfo[];
  defaultAgyDir: string;
  onBack: () => void;
  onAddAccount: (params: {
    agent: AgentKind;
    name: string;
    configDir?: string | null;
  }) => Promise<void>;
  onRenameAccount: (id: string, name: string) => Promise<void>;
  onRemoveAccount: (id: string, deleteDir: boolean) => Promise<void>;
  onInstallHooks: (id: string) => Promise<void>;
  onInstallDefaultAgyHooks: (configDir: string) => Promise<void>;
  onUninstallHooks: (id: string) => Promise<void>;
}

interface OverflowMenuState {
  account: AccountInfo;
  left: number;
  top: number;
}

function shortenPath(path: string): string {
  const match = path.match(/^\/home\/[^/]+(.*)$/);
  return match ? `~${match[1]}` : path;
}

function clampMenu(x: number, y: number, width: number, height: number) {
  return {
    left: Math.max(8, Math.min(x, window.innerWidth - width - 8)),
    top: Math.max(8, Math.min(y, window.innerHeight - height - 8)),
  };
}

export function AccountsView({
  accounts,
  defaultAgyDir,
  onBack,
  onAddAccount,
  onRenameAccount,
  onRemoveAccount,
  onInstallHooks,
  onInstallDefaultAgyHooks,
  onUninstallHooks,
}: AccountsViewProps) {
  const [addOpen, setAddOpen] = useState(false);
  const [addPrefillDir, setAddPrefillDir] = useState<string | null>(null);
  const [discoverOpen, setDiscoverOpen] = useState(false);
  const [installTarget, setInstallTarget] = useState<AccountInfo | null>(null);
  const [renameTarget, setRenameTarget] = useState<AccountInfo | null>(null);
  const [removeTarget, setRemoveTarget] = useState<AccountInfo | null>(null);
  const [uninstallingId, setUninstallingId] = useState<string | null>(null);
  const [overflowMenu, setOverflowMenu] = useState<OverflowMenuState | null>(null);

  useEffect(() => {
    if (!overflowMenu) return;
    const close = () => setOverflowMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [overflowMenu]);

  useEffect(() => {
    const anyModal =
      addOpen ||
      discoverOpen ||
      installTarget !== null ||
      renameTarget !== null ||
      removeTarget !== null ||
      overflowMenu !== null;
    if (anyModal) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onBack();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [addOpen, discoverOpen, installTarget, renameTarget, removeTarget, overflowMenu, onBack]);

  const claudeAccounts = accounts.filter((a) => a.agent === "claude");
  const codexAccounts = accounts.filter((a) => a.agent === "codex");
  const agyAccounts = accounts.filter((a) => a.agent === "agy");

  function renderAccountCard(account: AccountInfo, opts?: { isAgy?: boolean }) {
    const sourceLabel = account.owned
      ? es.accounts.sourceOwned
      : es.accounts.sourceExisting;
    const subtitle = opts?.isAgy
      ? es.accounts.agySubtitle
      : `${shortenPath(account.config_dir)} · ${sourceLabel}`;
    const isUninstalling = uninstallingId === account.id;

    return (
      <div
        key={account.id}
        className="pas-acc-card"
        data-account-id={account.id}
      >
        <div className="pas-acc-info">
          <b>{account.name}</b>
          <small title={account.config_dir}>{subtitle}</small>
        </div>

        <span className={`pas-tag${account.hooks_installed ? " is-ok" : ""}`}>
          {account.hooks_installed
            ? es.accounts.hooksInstalled
            : es.accounts.noHooks}
        </span>

        {account.hooks_installed ? (
          <button
            type="button"
            className="pas-btn pas-btn-sm"
            disabled={isUninstalling}
            onClick={() => {
              setUninstallingId(account.id);
              void onUninstallHooks(account.id).finally(() =>
                setUninstallingId((cur) => (cur === account.id ? null : cur)),
              );
            }}
          >
            {isUninstalling
              ? es.accounts.uninstallingHooks
              : es.accounts.uninstallHooks}
          </button>
        ) : (
          <button
            type="button"
            className="pas-btn pas-btn-sm is-primary"
            onClick={() => setInstallTarget(account)}
          >
            {es.accounts.installHooks}
          </button>
        )}

        {!opts?.isAgy && (
          <button
            type="button"
            className="pas-btn pas-btn-icon"
            title={es.accounts.moreActions}
            aria-label={es.accounts.moreActions}
            aria-haspopup="menu"
            aria-expanded={overflowMenu?.account.id === account.id}
            onClick={(e) => {
              e.stopPropagation();
              const rect = e.currentTarget.getBoundingClientRect();
              const pos = clampMenu(rect.right - 180, rect.bottom + 4, 180, 90);
              setOverflowMenu((cur) =>
                cur?.account.id === account.id
                  ? null
                  : { account, left: pos.left, top: pos.top },
              );
            }}
          >
            <MoreHorizontal size={14} strokeWidth={2} />
          </button>
        )}
      </div>
    );
  }

  return (
    <section className="pas-accounts-view" aria-label={es.accounts.title}>
      <div className="pas-accounts-inner">
        <div className="pas-accounts-header">
          <h2>{es.accounts.title}</h2>
          <button
            type="button"
            className="pas-btn pas-btn-sm pas-btn-inline"
            onClick={onBack}
          >
            <ArrowLeft size={13} strokeWidth={2.2} />
            <span>{es.accounts.back}</span>
          </button>
        </div>

        <div className="pas-acc-group">
          <div className="pas-acc-group-title">{es.newSessionDialog.agents.claude}</div>
          {claudeAccounts.length === 0 ? (
            <p className="pas-acc-empty">
              {es.accounts.emptyAgent(es.newSessionDialog.agents.claude)}
            </p>
          ) : (
            claudeAccounts.map((acc) => renderAccountCard(acc))
          )}
        </div>

        <div className="pas-acc-group">
          <div className="pas-acc-group-title">{es.newSessionDialog.agents.codex}</div>
          {codexAccounts.length === 0 ? (
            <p className="pas-acc-empty">
              {es.accounts.emptyAgent(es.newSessionDialog.agents.codex)}
            </p>
          ) : (
            codexAccounts.map((acc) => renderAccountCard(acc))
          )}
        </div>

        <div className="pas-acc-group" data-agent-group="agy">
          <div className="pas-acc-group-title">{es.newSessionDialog.agents.agy}</div>
          {agyAccounts.length > 0
            ? agyAccounts.map((acc) => renderAccountCard(acc, { isAgy: true }))
            : renderAccountCard(
                {
                  id: VIRTUAL_AGY_ID,
                  agent: "agy",
                  name: es.accounts.agyDefaultName,
                  config_dir: defaultAgyDir,
                  owned: false,
                  created_at: 0,
                  hooks_installed: false,
                },
                { isAgy: true },
              )}
        </div>

        <div className="pas-accounts-footer">
          <button
            type="button"
            className="pas-btn pas-btn-inline"
            onClick={() => {
              setAddPrefillDir(null);
              setAddOpen(true);
            }}
          >
            <Plus size={14} strokeWidth={2.2} />
            <span>{es.accounts.addAccount}</span>
          </button>
          <button
            type="button"
            className="pas-btn pas-btn-inline"
            onClick={() => setDiscoverOpen(true)}
          >
            <Search size={14} strokeWidth={2} />
            <span>{es.accounts.detectAccounts}</span>
          </button>
        </div>
      </div>

      {overflowMenu && (
        <div
          className="pas-menu pas-acc-overflow-menu"
          role="menu"
          style={{ left: overflowMenu.left, top: overflowMenu.top }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item"
            onClick={() => {
              const target = overflowMenu.account;
              setOverflowMenu(null);
              setRenameTarget(target);
            }}
          >
            {es.accounts.rename}
          </button>
          <div className="pas-menu-sep" />
          <button
            type="button"
            role="menuitem"
            className="pas-menu-item is-danger"
            onClick={() => {
              const target = overflowMenu.account;
              setOverflowMenu(null);
              setRemoveTarget(target);
            }}
          >
            {es.accounts.remove}
          </button>
        </div>
      )}

      <AddAccountDialog
        open={addOpen}
        initialExistingDir={addPrefillDir}
        onClose={() => {
          setAddOpen(false);
          setAddPrefillDir(null);
        }}
        onSubmit={onAddAccount}
      />

      <DiscoverAccountsDialog
        open={discoverOpen}
        onClose={() => setDiscoverOpen(false)}
        onAddAccount={onAddAccount}
        onChooseFolder={(dir) => {
          setDiscoverOpen(false);
          setAddPrefillDir(dir);
          setAddOpen(true);
        }}
      />

      <InstallHooksDialog
        account={installTarget}
        onClose={() => setInstallTarget(null)}
        onConfirm={async (account) => {
          if (account.id === VIRTUAL_AGY_ID) {
            await onInstallDefaultAgyHooks(account.config_dir);
          } else {
            await onInstallHooks(account.id);
          }
        }}
      />

      <TextPromptDialog
        open={renameTarget !== null}
        title={es.accounts.renameDialogTitle}
        initialValue={renameTarget?.name ?? ""}
        submitLabel={es.renameDialog.submit}
        onClose={() => setRenameTarget(null)}
        onSubmit={async (newName) => {
          if (!renameTarget) return;
          await onRenameAccount(renameTarget.id, newName);
        }}
      />

      <RemoveAccountDialog
        account={removeTarget}
        onClose={() => setRemoveTarget(null)}
        onConfirm={async (account, deleteDir) => {
          await onRemoveAccount(account.id, deleteDir);
        }}
      />
    </section>
  );
}

interface AddAccountDialogProps {
  open: boolean;
  initialExistingDir?: string | null;
  onClose: () => void;
  onSubmit: (params: {
    agent: AgentKind;
    name: string;
    configDir?: string | null;
  }) => Promise<void>;
}

function AddAccountDialog({
  open,
  initialExistingDir = null,
  onClose,
  onSubmit,
}: AddAccountDialogProps) {
  const [agent, setAgent] = useState<"claude" | "codex">("claude");
  const [name, setName] = useState("");
  const [dirMode, setDirMode] = useState<"new" | "existing">("new");
  const [existingDir, setExistingDir] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setAgent("claude");
    setName("");
    setDirMode(initialExistingDir ? "existing" : "new");
    setExistingDir(initialExistingDir ?? "");
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
  }, [open, initialExistingDir, onClose]);

  if (!open) return null;

  async function handleChooseDir() {
    try {
      const chosen = await openDialog({ directory: true, multiple: false });
      if (typeof chosen === "string" && chosen.trim()) {
        setExistingDir(chosen);
        setError(null);
      }
    } catch (err) {
      setError(String(err));
    }
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError(es.accounts.addDialog.errorEmptyName);
      return;
    }
    const trimmedDir = existingDir.trim();
    if (dirMode === "existing" && !trimmedDir) {
      setError(es.accounts.addDialog.errorEmptyDir);
      return;
    }

    setError(null);
    setSubmitting(true);
    try {
      await onSubmit({
        agent,
        name: trimmedName,
        configDir: dirMode === "existing" ? trimmedDir : null,
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
      aria-labelledby="add-account-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <form className="pas-dialog-card is-wide" onSubmit={handleSubmit}>
        <h4 id="add-account-title">{es.accounts.addDialog.title}</h4>

        <div className="pas-field">
          <label>{es.accounts.addDialog.agentLabel}</label>
          <div className="pas-seg" role="radiogroup">
            {(["claude", "codex"] as const).map((kind) => (
              <button
                key={kind}
                type="button"
                role="radio"
                aria-checked={agent === kind}
                className={`pas-seg-btn${agent === kind ? " is-active" : ""}`}
                onClick={() => setAgent(kind)}
              >
                {es.newSessionDialog.agents[kind]}
              </button>
            ))}
          </div>
        </div>

        <div className="pas-field">
          <label htmlFor="acc-name">{es.accounts.addDialog.nameLabel}</label>
          <input
            id="acc-name"
            className="pas-input"
            type="text"
            autoFocus
            value={name}
            placeholder={es.accounts.addDialog.namePlaceholder}
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div className="pas-field">
          <label>{es.accounts.addDialog.dirModeLabel}</label>
          <div className="pas-seg" role="radiogroup">
            <button
              type="button"
              role="radio"
              aria-checked={dirMode === "new"}
              className={`pas-seg-btn${dirMode === "new" ? " is-active" : ""}`}
              onClick={() => setDirMode("new")}
            >
              {es.accounts.addDialog.dirModeNew}
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={dirMode === "existing"}
              className={`pas-seg-btn${dirMode === "existing" ? " is-active" : ""}`}
              onClick={() => setDirMode("existing")}
            >
              {es.accounts.addDialog.dirModeExisting}
            </button>
          </div>
        </div>

        {dirMode === "new" ? (
          <p className="pas-field-hint">{es.accounts.addDialog.dirModeNewHint}</p>
        ) : (
          <div className="pas-field">
            <label htmlFor="acc-dir">{es.accounts.addDialog.existingDirLabel}</label>
            <div className="pas-path-row">
              <input
                id="acc-dir"
                className="pas-input pas-input-mono"
                type="text"
                value={existingDir}
                placeholder={es.accounts.addDialog.existingDirPlaceholder}
                onChange={(e) => setExistingDir(e.target.value)}
              />
              <button
                type="button"
                className="pas-btn pas-btn-inline"
                onClick={() => void handleChooseDir()}
              >
                <FolderOpen size={13} strokeWidth={2} />
                <span>{es.accounts.chooseFolder}</span>
              </button>
            </div>
          </div>
        )}

        {error && <p className="pas-field-error">{error}</p>}

        <div className="pas-dialog-actions">
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.accounts.addDialog.cancel}
          </button>
          <button
            type="submit"
            className="pas-btn is-primary"
            disabled={submitting}
          >
            {submitting
              ? es.accounts.addDialog.submitting
              : es.accounts.addDialog.submit}
          </button>
        </div>
      </form>
    </div>
  );
}

function agentLabel(agent: AgentKind): string {
  if (agent === "claude") return es.newSessionDialog.agents.claude;
  if (agent === "codex") return es.newSessionDialog.agents.codex;
  return es.newSessionDialog.agents.agy;
}

function discoveredAccountSubtitle(item: DiscoveredAccount): string {
  if (item.already_added_as) {
    return es.accounts.discoverDialog.alreadyAddedAs(item.already_added_as);
  }
  if (item.config_dir.includes("/claude-profiles/")) {
    return es.accounts.discoverDialog.orcaProfile;
  }
  return shortenPath(item.config_dir);
}

interface DiscoverAccountsDialogProps {
  open: boolean;
  onClose: () => void;
  onAddAccount: (params: {
    agent: AgentKind;
    name: string;
    configDir?: string | null;
  }) => Promise<void>;
  onChooseFolder: (dir: string) => void;
}

function DiscoverAccountsDialog({
  open,
  onClose,
  onAddAccount,
  onChooseFolder,
}: DiscoverAccountsDialogProps) {
  const [items, setItems] = useState<DiscoveredAccount[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedDirs, setSelectedDirs] = useState<Set<string>>(() => new Set());
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setSubmitting(false);

    let cancelled = false;
    setLoading(true);
    void api
      .discoverAccounts()
      .then((list) => {
        if (cancelled) return;
        setItems(list);
        setSelectedDirs(
          new Set(
            list.filter((a) => !a.already_added_as).map((a) => a.config_dir),
          ),
        );
      })
      .catch((err) => {
        if (!cancelled) {
          setItems([]);
          setSelectedDirs(new Set());
          setError(String(err));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
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

  function toggleItem(item: DiscoveredAccount) {
    if (item.already_added_as) return;
    setSelectedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(item.config_dir)) next.delete(item.config_dir);
      else next.add(item.config_dir);
      return next;
    });
  }

  async function handleChooseFolder() {
    try {
      const chosen = await openDialog({ directory: true, multiple: false });
      if (typeof chosen === "string" && chosen.trim()) {
        onChooseFolder(chosen);
      }
    } catch (err) {
      setError(String(err));
    }
  }

  const toAdd = items.filter(
    (item) => !item.already_added_as && selectedDirs.has(item.config_dir),
  );

  async function handleSubmit() {
    if (toAdd.length === 0 || submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      for (const item of toAdd) {
        await onAddAccount({
          agent: item.agent,
          name: item.suggested_name,
          configDir: item.config_dir,
        });
      }
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
      aria-labelledby="discover-accounts-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="pas-dialog-card is-discover-accounts">
        <h4 id="discover-accounts-title">{es.accounts.discoverDialog.title}</h4>

        {loading ? (
          <div className="pas-sug-empty">{es.accounts.discoverDialog.loading}</div>
        ) : items.length === 0 ? (
          <div className="pas-sug-empty">{es.accounts.discoverDialog.empty}</div>
        ) : (
          <div className="pas-sug" role="listbox" aria-multiselectable="true">
            {items.map((item) => {
              const disabled = Boolean(item.already_added_as);
              const selected = !disabled && selectedDirs.has(item.config_dir);
              return (
                <button
                  key={item.config_dir}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  disabled={disabled}
                  className={`pas-sug-row${selected ? " is-selected" : ""}${disabled ? " is-disabled" : ""}`}
                  onClick={() => toggleItem(item)}
                >
                  <span
                    className={`pas-sug-box${selected ? " is-checked" : ""}`}
                  >
                    {selected && <Check size={10} strokeWidth={3} />}
                  </span>
                  <span className="pas-sug-name">
                    <b className="pas-sug-agent">{agentLabel(item.agent)}</b>
                    <span className="pas-sug-sub">
                      {" · "}
                      {discoveredAccountSubtitle(item)}
                    </span>
                  </span>
                  <span className="pas-sug-path" title={item.config_dir}>
                    {shortenPath(item.config_dir)}
                  </span>
                </button>
              );
            })}
          </div>
        )}

        <p className="pas-sug-hint">{es.accounts.discoverDialog.hint}</p>

        {error && <p className="pas-field-error">{error}</p>}

        <div className="pas-dialog-actions is-split">
          <button
            type="button"
            className="pas-btn pas-btn-inline"
            onClick={() => void handleChooseFolder()}
          >
            <FolderOpen size={13} strokeWidth={2} />
            <span>{es.accounts.discoverDialog.chooseFolder}</span>
          </button>
          <span className="pas-spacer" />
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.accounts.discoverDialog.cancel}
          </button>
          <button
            type="button"
            className="pas-btn is-primary"
            disabled={submitting || toAdd.length === 0}
            onClick={() => void handleSubmit()}
          >
            {submitting
              ? es.accounts.discoverDialog.submitting
              : es.accounts.discoverDialog.submitCount(toAdd.length)}
          </button>
        </div>
      </div>
    </div>
  );
}

interface InstallHooksDialogProps {
  account: AccountInfo | null;
  onClose: () => void;
  onConfirm: (account: AccountInfo) => Promise<void>;
}

function InstallHooksDialog({
  account,
  onClose,
  onConfirm,
}: InstallHooksDialogProps) {
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!account) return;
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
  }, [account, onClose]);

  if (!account) return null;

  const target = hooksTargetInfo(account);
  const label = quotaPillLabel(account);

  async function handleConfirm() {
    if (!account) return;
    setError(null);
    setSubmitting(true);
    try {
      await onConfirm(account);
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
      aria-labelledby="install-hooks-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="pas-dialog-card is-hooks">
        <h4 id="install-hooks-title">{es.accounts.hooksDialog.title(label)}</h4>
        <p className="pas-hooks-intro">
          {target.wrapsStatusline
            ? es.accounts.hooksDialog.introWithStatusline
            : es.accounts.hooksDialog.introHooksOnly}
        </p>
        <div className="pas-hooks-path" title={target.filePath}>
          {target.filePath}
        </div>
        <ul className="pas-hooks-list">
          <li>{es.accounts.hooksDialog.bulletBackup(target.backupName)}</li>
          <li>{es.accounts.hooksDialog.bulletForeignUntouched}</li>
          <li>{es.accounts.hooksDialog.bulletNoopOutside}</li>
        </ul>

        {error && <p className="pas-field-error">{error}</p>}

        <div className="pas-dialog-actions">
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.accounts.hooksDialog.cancel}
          </button>
          <button
            type="button"
            className="pas-btn is-primary"
            disabled={submitting}
            onClick={() => void handleConfirm()}
          >
            {submitting
              ? es.accounts.hooksDialog.submitting
              : es.accounts.hooksDialog.submit}
          </button>
        </div>
      </div>
    </div>
  );
}

interface RemoveAccountDialogProps {
  account: AccountInfo | null;
  onClose: () => void;
  onConfirm: (account: AccountInfo, deleteDir: boolean) => Promise<void>;
}

function RemoveAccountDialog({
  account,
  onClose,
  onConfirm,
}: RemoveAccountDialogProps) {
  const [deleteDir, setDeleteDir] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!account) return;
    setDeleteDir(false);
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
  }, [account, onClose]);

  if (!account) return null;

  async function handleConfirm() {
    if (!account) return;
    setError(null);
    setSubmitting(true);
    try {
      await onConfirm(account, account.owned && deleteDir);
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
      aria-labelledby="remove-account-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="pas-dialog-card">
        <h4 id="remove-account-title">
          {es.accounts.removeDialog.title(account.name)}
        </h4>
        <p className="pas-hooks-intro">
          {account.owned
            ? es.accounts.removeDialog.bodyOwned
            : es.accounts.removeDialog.bodyExisting}
        </p>
        {account.owned && (
          <label className="pas-check">
            <input
              type="checkbox"
              checked={deleteDir}
              onChange={(e) => setDeleteDir(e.target.checked)}
            />
            <span>{es.accounts.removeDialog.deleteDirLabel}</span>
          </label>
        )}
        {error && <p className="pas-field-error">{error}</p>}
        <div className="pas-dialog-actions">
          <button type="button" className="pas-btn" onClick={onClose}>
            {es.accounts.removeDialog.cancel}
          </button>
          <button
            type="button"
            className="pas-btn is-danger"
            disabled={submitting}
            onClick={() => void handleConfirm()}
          >
            {submitting
              ? es.accounts.removeDialog.submitting
              : es.accounts.removeDialog.submit}
          </button>
        </div>
      </div>
    </div>
  );
}
