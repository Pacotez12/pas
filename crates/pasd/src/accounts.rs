use std::fs::{self, Permissions};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use pas_core::protocol::{AccountAddParams, AccountInfo, AgentKind};

use crate::hooks_install::{self, HooksInstallError};
use crate::store::{Store, StoreError};

fn with_hooks_status(mut account: AccountInfo) -> AccountInfo {
    account.hooks_installed = hooks_install::is_installed(account.agent, &account.config_dir);
    account
}

pub fn list_accounts(store: &Store) -> Result<Vec<AccountInfo>, StoreError> {
    let list = store.list_accounts()?;
    Ok(list.into_iter().map(with_hooks_status).collect())
}

pub fn add_account(
    store: &Store,
    accounts_dir: Option<&Path>,
    params: AccountAddParams,
) -> Result<AccountInfo, StoreError> {
    let name = params.name.trim();
    if name.is_empty() {
        return Err(StoreError::EmptyName { kind: "cuenta" });
    }

    match params.config_dir {
        None => {
            let base_dir = accounts_dir
                .or_else(|| store.default_accounts_dir())
                .ok_or(StoreError::NoAccountsDir)?;

            let id = store.generate_account_id()?;
            let dir = base_dir.join(&id);
            fs::create_dir_all(&dir)?;
            fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
            let canonical = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());

            match store.insert_account(&id, params.agent, name, &canonical, true) {
                Ok(info) => Ok(with_hooks_status(info)),
                Err(err) => {
                    let _ = fs::remove_dir_all(&dir);
                    Err(err)
                }
            }
        }
        Some(dir) => {
            if !dir.is_dir() {
                return Err(StoreError::AccountDirInvalid { path: dir });
            }
            let canonical = fs::canonicalize(&dir)?;
            let id = store.generate_account_id()?;
            store
                .insert_account(&id, params.agent, name, &canonical, false)
                .map(with_hooks_status)
        }
    }
}

pub fn rename_account(store: &Store, id: &str, name: &str) -> Result<AccountInfo, StoreError> {
    store.rename_account(id, name).map(with_hooks_status)
}

#[derive(Debug, thiserror::Error)]
pub enum AccountHooksError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Hooks(#[from] HooksInstallError),
}

pub fn install_account_hooks(store: &Store, id: &str) -> Result<AccountInfo, AccountHooksError> {
    let account = store.get_account(id)?;
    let pas_bin = hooks_install::find_pas_bin()?;
    let snapshots = hooks_install::install(account.agent, &account.config_dir, &pas_bin)?;
    store.save_hook_snapshots(id, &snapshots)?;
    Ok(with_hooks_status(account))
}

pub fn uninstall_account_hooks(store: &Store, id: &str) -> Result<AccountInfo, AccountHooksError> {
    let account = store.get_account(id)?;
    let snapshots = store.load_hook_snapshots(id)?;
    hooks_install::uninstall(account.agent, &account.config_dir, &snapshots)?;
    store.delete_hook_snapshots(id)?;
    Ok(with_hooks_status(account))
}

pub fn remove_account(store: &Store, id: &str, delete_dir: bool) -> Result<(), StoreError> {
    let account = store.delete_account(id)?;
    if account.owned
        && delete_dir
        && let Err(err) = fs::remove_dir_all(&account.config_dir)
        && err.kind() != ErrorKind::NotFound
    {
        return Err(StoreError::Io(err));
    }
    Ok(())
}

pub fn env_for_account(account: &AccountInfo) -> Option<(&'static str, String)> {
    let dir = account.config_dir.to_string_lossy().into_owned();
    match account.agent {
        AgentKind::Claude => Some(("CLAUDE_CONFIG_DIR", dir)),
        AgentKind::Codex => Some(("CODEX_HOME", dir)),
        AgentKind::Agy => None,
    }
}
