use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use pas_core::protocol::{
    AccountInfo, AgentKind, GroupInfo, ProjectInfo, WorktreeInfo, WorktreeState,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::hooks_install::FileSnapshot;
use crate::remote::{read_origin_remote_key, repo_name_from_remote_key};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{} no es un repositorio git. Elegí la carpeta raíz del repo.", path.display())]
    NotAGitRepo { path: PathBuf },

    #[error(
        "El proyecto en {} ya está agregado. Usá otro directorio o quitá el existente primero.",
        path.display()
    )]
    ProjectAlreadyExists { path: PathBuf },

    #[error("No hay ningún proyecto con id {id}. Revisá la lista de proyectos.")]
    ProjectNotFound { id: String },

    #[error("No hay ningún grupo con id {id}. Revisá la lista de grupos.")]
    GroupNotFound { id: String },

    #[error("Ya existe un grupo llamado {name}. Elegí otro nombre.")]
    GroupAlreadyExists { name: String },

    #[error("No se encontró esa cuenta. Revisá la lista de cuentas disponibles.")]
    AccountNotFound,

    #[error("No se encontró ese worktree. Revisá la lista de worktrees del proyecto.")]
    WorktreeNotFound,

    #[error(
        "Ya existe una cuenta que apunta a {}. Dos cuentas de pas no pueden compartir directorio; elegí otra carpeta.",
        config_dir.display()
    )]
    AccountAlreadyExists { config_dir: PathBuf },

    #[error(
        "La carpeta {} no existe o no es un directorio. Creala o elegí una carpeta existente.",
        path.display()
    )]
    AccountDirInvalid { path: PathBuf },

    #[error(
        "No hay una carpeta de datos configurada para crear cuentas nuevas. Indicá una carpeta existente o configurá --data-dir."
    )]
    NoAccountsDir,

    #[error("El nombre del {kind} no puede estar vacío. Escribí un nombre válido.")]
    EmptyName { kind: &'static str },

    #[error("Error guardando datos en SQLite: {0}. Revisá los permisos de la carpeta de datos.")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Error accediendo a archivos: {0}.")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
    accounts_dir: Option<PathBuf>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let accounts_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.join("accounts"));
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        init_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            accounts_dir,
        })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        init_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            accounts_dir: None,
        })
    }

    pub fn default_accounts_dir(&self) -> Option<&Path> {
        self.accounts_dir.as_deref()
    }

    pub fn add_project(&self, path: &Path, name: Option<&str>) -> Result<ProjectInfo, StoreError> {
        if !path.join(".git").exists() {
            return Err(StoreError::NotAGitRepo {
                path: path.to_path_buf(),
            });
        }

        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let project_name = name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                canonical
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "proyecto".into())
            });

        let remote_key = read_origin_remote_key(&canonical);
        let path_str = canonical.to_string_lossy().into_owned();
        let created_at = now_unix_secs();

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM projects WHERE path = ?1",
                params![&path_str],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(StoreError::ProjectAlreadyExists { path: canonical });
        }

        let group_id = match remote_key {
            Some(rk) => find_or_create_remote_group(&tx, &rk, &project_name)?,
            None => create_unique_group(&tx, &project_name, None)?,
        };

        let id = unique_id(&tx, "projects")?;
        tx.execute(
            "INSERT INTO projects (id, name, path, group_id, group_pinned, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![&id, &project_name, &path_str, &group_id, created_at],
        )?;
        tx.commit()?;

        Ok(ProjectInfo {
            id,
            name: project_name,
            path: canonical,
            group_id,
            group_pinned: false,
        })
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, path, group_id, group_pinned
             FROM projects
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], map_project_row)?;
        let mut projects = Vec::new();
        for row in rows {
            projects.push(row?);
        }
        Ok(projects)
    }

    pub fn get_project(&self, id: &str) -> Result<ProjectInfo, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.query_row(
            "SELECT id, name, path, group_id, group_pinned FROM projects WHERE id = ?1",
            params![id],
            map_project_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::ProjectNotFound { id: id.to_string() })
    }

    pub fn find_project_for_cwd(&self, cwd: &Path) -> Result<Option<ProjectInfo>, StoreError> {
        let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        let projects = self.list_projects()?;
        Ok(projects
            .into_iter()
            .filter_map(|project| {
                let canonical_project =
                    std::fs::canonicalize(&project.path).unwrap_or_else(|_| project.path.clone());
                if canonical_cwd.starts_with(&canonical_project) {
                    let depth = canonical_project.components().count();
                    let len = canonical_project.as_os_str().len();
                    Some(((depth, len), project))
                } else {
                    None
                }
            })
            .max_by_key(|(key, _)| *key)
            .map(|(_, project)| project))
    }

    pub fn remove_project(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let group_id: Option<String> = tx
            .query_row(
                "SELECT group_id FROM projects WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(group_id) = group_id else {
            return Err(StoreError::ProjectNotFound { id: id.to_string() });
        };

        tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        delete_group_if_empty(&tx, &group_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn rename_project(&self, id: &str, name: &str) -> Result<ProjectInfo, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::EmptyName { kind: "proyecto" });
        }

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let updated = tx.execute(
            "UPDATE projects SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        if updated == 0 {
            return Err(StoreError::ProjectNotFound { id: id.to_string() });
        }

        let project = fetch_project(&tx, id)?;
        tx.commit()?;
        Ok(project)
    }

    pub fn move_project(&self, id: &str, group_id: &str) -> Result<ProjectInfo, StoreError> {
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let current = fetch_project(&tx, id)?;
        let target_exists: Option<String> = tx
            .query_row(
                "SELECT id FROM groups WHERE id = ?1",
                params![group_id],
                |row| row.get(0),
            )
            .optional()?;
        if target_exists.is_none() {
            return Err(StoreError::GroupNotFound {
                id: group_id.to_string(),
            });
        }

        tx.execute(
            "UPDATE projects SET group_id = ?1, group_pinned = 1 WHERE id = ?2",
            params![group_id, id],
        )?;

        if current.group_id != group_id {
            delete_group_if_empty(&tx, &current.group_id)?;
        }

        let updated = fetch_project(&tx, id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn unpin_project(&self, id: &str) -> Result<ProjectInfo, StoreError> {
        let project_path = {
            let conn = self.conn.lock().expect("store lock poisoned");
            let path: Option<String> = conn
                .query_row(
                    "SELECT path FROM projects WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;
            match path {
                Some(p) => PathBuf::from(p),
                None => return Err(StoreError::ProjectNotFound { id: id.to_string() }),
            }
        };

        let remote_key = read_origin_remote_key(&project_path);

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let current = fetch_project(&tx, id)?;
        let target_group_id = match remote_key {
            Some(rk) => find_or_create_remote_group(&tx, &rk, &current.name)?,
            None => {
                let current_is_sole_non_remote: bool = {
                    let rk: Option<String> = tx
                        .query_row(
                            "SELECT remote_key FROM groups WHERE id = ?1",
                            params![&current.group_id],
                            |row| row.get(0),
                        )
                        .optional()?
                        .flatten();
                    let count: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM projects WHERE group_id = ?1",
                        params![&current.group_id],
                        |row| row.get(0),
                    )?;
                    rk.is_none() && count == 1
                };
                if current_is_sole_non_remote {
                    current.group_id.clone()
                } else {
                    create_unique_group(&tx, &current.name, None)?
                }
            }
        };

        tx.execute(
            "UPDATE projects SET group_id = ?1, group_pinned = 0 WHERE id = ?2",
            params![&target_group_id, id],
        )?;

        if current.group_id != target_group_id {
            delete_group_if_empty(&tx, &current.group_id)?;
        }

        let updated = fetch_project(&tx, id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn list_groups(&self) -> Result<Vec<GroupInfo>, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, remote_key
             FROM groups
             ORDER BY name ASC, id ASC",
        )?;
        let rows = stmt.query_map([], map_group_row)?;
        let mut groups = Vec::new();
        for row in rows {
            groups.push(row?);
        }
        Ok(groups)
    }

    pub fn create_group(&self, name: &str) -> Result<GroupInfo, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::EmptyName { kind: "grupo" });
        }

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM groups WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(StoreError::GroupAlreadyExists {
                name: name.to_string(),
            });
        }

        let id = unique_id(&tx, "groups")?;
        tx.execute(
            "INSERT INTO groups (id, name, remote_key) VALUES (?1, ?2, NULL)",
            params![&id, name],
        )?;
        tx.commit()?;

        Ok(GroupInfo {
            id,
            name: name.to_string(),
            remote_key: None,
        })
    }

    pub fn rename_group(&self, id: &str, name: &str) -> Result<GroupInfo, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::EmptyName { kind: "grupo" });
        }

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let existing: Option<GroupInfo> = tx
            .query_row(
                "SELECT id, name, remote_key FROM groups WHERE id = ?1",
                params![id],
                map_group_row,
            )
            .optional()?;
        let Some(existing) = existing else {
            return Err(StoreError::GroupNotFound { id: id.to_string() });
        };

        let conflict: Option<String> = tx
            .query_row(
                "SELECT id FROM groups WHERE name = ?1 AND id != ?2",
                params![name, id],
                |row| row.get(0),
            )
            .optional()?;
        if conflict.is_some() {
            return Err(StoreError::GroupAlreadyExists {
                name: name.to_string(),
            });
        }

        tx.execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        tx.commit()?;

        Ok(GroupInfo {
            id: existing.id,
            name: name.to_string(),
            remote_key: existing.remote_key,
        })
    }

    pub fn generate_account_id(&self) -> Result<String, StoreError> {
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;
        unique_id(&tx, "accounts")
    }

    pub fn insert_account(
        &self,
        id: &str,
        agent: AgentKind,
        name: &str,
        config_dir: &Path,
        owned: bool,
    ) -> Result<AccountInfo, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::EmptyName { kind: "cuenta" });
        }

        let dir_str = config_dir.to_string_lossy().into_owned();
        let created_at = now_unix_secs();

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM accounts WHERE config_dir = ?1",
                params![&dir_str],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(StoreError::AccountAlreadyExists {
                config_dir: config_dir.to_path_buf(),
            });
        }

        tx.execute(
            "INSERT INTO accounts (id, agent, name, config_dir, owned, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                agent.as_str(),
                name,
                &dir_str,
                if owned { 1i64 } else { 0i64 },
                created_at
            ],
        )?;
        tx.commit()?;

        Ok(AccountInfo {
            id: id.to_string(),
            agent,
            name: name.to_string(),
            config_dir: config_dir.to_path_buf(),
            owned,
            created_at: created_at.max(0) as u64,
            hooks_installed: false,
        })
    }

    pub fn list_accounts(&self) -> Result<Vec<AccountInfo>, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, agent, name, config_dir, owned, created_at
             FROM accounts
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], map_account_row)?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row?);
        }
        Ok(accounts)
    }

    pub fn get_account(&self, id: &str) -> Result<AccountInfo, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.query_row(
            "SELECT id, agent, name, config_dir, owned, created_at
             FROM accounts
             WHERE id = ?1",
            params![id],
            map_account_row,
        )
        .optional()?
        .ok_or(StoreError::AccountNotFound)
    }

    pub fn rename_account(&self, id: &str, name: &str) -> Result<AccountInfo, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::EmptyName { kind: "cuenta" });
        }

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let updated = tx.execute(
            "UPDATE accounts SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        if updated == 0 {
            return Err(StoreError::AccountNotFound);
        }

        let account = tx.query_row(
            "SELECT id, agent, name, config_dir, owned, created_at
             FROM accounts
             WHERE id = ?1",
            params![id],
            map_account_row,
        )?;
        tx.commit()?;
        Ok(account)
    }

    pub fn delete_account(&self, id: &str) -> Result<AccountInfo, StoreError> {
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;

        let existing: Option<AccountInfo> = tx
            .query_row(
                "SELECT id, agent, name, config_dir, owned, created_at
                 FROM accounts
                 WHERE id = ?1",
                params![id],
                map_account_row,
            )
            .optional()?;
        let Some(existing) = existing else {
            return Err(StoreError::AccountNotFound);
        };

        tx.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(existing)
    }

    pub fn save_hook_snapshots(
        &self,
        account_id: &str,
        snapshots: &[FileSnapshot],
    ) -> Result<(), StoreError> {
        if snapshots.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;
        for snap in snapshots {
            let path_str = snap.path.to_string_lossy().into_owned();
            tx.execute(
                "INSERT INTO hook_snapshots (account_id, file_path, original_bytes, installed_bytes)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, file_path) DO UPDATE SET
                     original_bytes = excluded.original_bytes,
                     installed_bytes = excluded.installed_bytes",
                params![
                    account_id,
                    &path_str,
                    snap.original_bytes.as_deref(),
                    &snap.installed_bytes
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_hook_snapshots(&self, account_id: &str) -> Result<Vec<FileSnapshot>, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_path, original_bytes, installed_bytes
             FROM hook_snapshots
             WHERE account_id = ?1",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            let path: String = row.get(0)?;
            let original_bytes: Option<Vec<u8>> = row.get(1)?;
            let installed_bytes: Vec<u8> = row.get(2)?;
            Ok(FileSnapshot {
                path: PathBuf::from(path),
                original_bytes,
                installed_bytes,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_hook_snapshots(&self, account_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "DELETE FROM hook_snapshots WHERE account_id = ?1",
            params![account_id],
        )?;
        Ok(())
    }

    pub fn insert_worktree(
        &self,
        project_id: &str,
        branch: &str,
        base: &str,
        path: &Path,
        session_id: Option<&str>,
    ) -> Result<WorktreeInfo, StoreError> {
        let path_str = path.to_string_lossy().into_owned();
        let created_at = now_unix_secs();

        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;
        let _ = fetch_project(&tx, project_id)?;
        let id = unique_id(&tx, "worktrees")?;
        tx.execute(
            "INSERT INTO worktrees (id, project_id, branch, base, path, session_id, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7)",
            params![&id, project_id, branch, base, &path_str, session_id, created_at],
        )?;
        tx.commit()?;

        Ok(WorktreeInfo {
            id,
            project_id: project_id.to_string(),
            branch: branch.to_string(),
            base: base.to_string(),
            path: path.to_path_buf(),
            session_id: session_id.map(ToOwned::to_owned),
            state: WorktreeState::Active,
            created_at: created_at.max(0) as u64,
        })
    }

    pub fn get_worktree(&self, id: &str) -> Result<WorktreeInfo, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.query_row(
            "SELECT id, project_id, branch, base, path, session_id, state, created_at
             FROM worktrees
             WHERE id = ?1",
            params![id],
            map_worktree_row,
        )
        .optional()?
        .ok_or(StoreError::WorktreeNotFound)
    }

    pub fn list_worktrees(&self, project_id: &str) -> Result<Vec<WorktreeInfo>, StoreError> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, branch, base, path, session_id, state, created_at
             FROM worktrees
             WHERE project_id = ?1 AND state != 'removed'
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![project_id], map_worktree_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn set_worktree_state(
        &self,
        id: &str,
        state: WorktreeState,
    ) -> Result<WorktreeInfo, StoreError> {
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE worktrees SET state = ?1 WHERE id = ?2",
            params![state.as_str(), id],
        )?;
        if updated == 0 {
            return Err(StoreError::WorktreeNotFound);
        }
        let info = tx.query_row(
            "SELECT id, project_id, branch, base, path, session_id, state, created_at
             FROM worktrees
             WHERE id = ?1",
            params![id],
            map_worktree_row,
        )?;
        tx.commit()?;
        Ok(info)
    }

    pub fn set_worktree_session(
        &self,
        id: &str,
        session_id: Option<&str>,
    ) -> Result<WorktreeInfo, StoreError> {
        let mut conn = self.conn.lock().expect("store lock poisoned");
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE worktrees SET session_id = ?1 WHERE id = ?2",
            params![session_id, id],
        )?;
        if updated == 0 {
            return Err(StoreError::WorktreeNotFound);
        }
        let info = tx.query_row(
            "SELECT id, project_id, branch, base, path, session_id, state, created_at
             FROM worktrees
             WHERE id = ?1",
            params![id],
            map_worktree_row,
        )?;
        tx.commit()?;
        Ok(info)
    }
}

fn init_connection(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS groups (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 remote_key TEXT UNIQUE
             );
             CREATE TABLE IF NOT EXISTS projects (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 path TEXT NOT NULL UNIQUE,
                 group_id TEXT NOT NULL REFERENCES groups(id),
                 group_pinned INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    if version < 2 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS accounts (
                 id TEXT PRIMARY KEY,
                 agent TEXT NOT NULL CHECK(agent IN ('claude', 'codex', 'agy')),
                 name TEXT NOT NULL,
                 config_dir TEXT NOT NULL UNIQUE,
                 owned INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL
             );
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
    }
    if version < 3 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS hook_snapshots (
                 account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                 file_path TEXT NOT NULL,
                 original_bytes BLOB,
                 installed_bytes BLOB NOT NULL,
                 PRIMARY KEY (account_id, file_path)
             );
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    if version < 4 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS worktrees (
                 id TEXT PRIMARY KEY,
                 project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                 branch TEXT NOT NULL,
                 base TEXT NOT NULL,
                 path TEXT NOT NULL,
                 session_id TEXT,
                 state TEXT NOT NULL CHECK(state IN ('active', 'merged', 'removed')),
                 created_at INTEGER NOT NULL
             );
             PRAGMA user_version = 4;
             COMMIT;",
        )?;
    }
    Ok(())
}

fn map_worktree_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorktreeInfo> {
    let path: String = row.get(4)?;
    let state_str: String = row.get(6)?;
    let state = WorktreeState::parse(&state_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            format!("estado inválido en worktrees: {state_str}").into(),
        )
    })?;
    let created_at: i64 = row.get(7)?;
    Ok(WorktreeInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        branch: row.get(2)?,
        base: row.get(3)?,
        path: PathBuf::from(path),
        session_id: row.get(5)?,
        state,
        created_at: created_at.max(0) as u64,
    })
}

fn map_account_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountInfo> {
    let agent_str: String = row.get(1)?;
    let agent = AgentKind::parse(&agent_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("agente inválido en accounts: {agent_str}").into(),
        )
    })?;
    let config_dir: String = row.get(3)?;
    let owned: i64 = row.get(4)?;
    let created_at: i64 = row.get(5)?;
    Ok(AccountInfo {
        id: row.get(0)?,
        agent,
        name: row.get(2)?,
        config_dir: PathBuf::from(config_dir),
        owned: owned != 0,
        created_at: created_at.max(0) as u64,
        hooks_installed: false,
    })
}

fn map_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectInfo> {
    let path: String = row.get(2)?;
    let pinned: i64 = row.get(4)?;
    Ok(ProjectInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        path: PathBuf::from(path),
        group_id: row.get(3)?,
        group_pinned: pinned != 0,
    })
}

fn map_group_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupInfo> {
    Ok(GroupInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        remote_key: row.get(2)?,
    })
}

fn fetch_project(tx: &Transaction<'_>, id: &str) -> Result<ProjectInfo, StoreError> {
    tx.query_row(
        "SELECT id, name, path, group_id, group_pinned FROM projects WHERE id = ?1",
        params![id],
        map_project_row,
    )
    .optional()?
    .ok_or_else(|| StoreError::ProjectNotFound { id: id.to_string() })
}

fn find_or_create_remote_group(
    tx: &Transaction<'_>,
    remote_key: &str,
    fallback_name: &str,
) -> Result<String, StoreError> {
    if let Some(existing_id) = tx
        .query_row(
            "SELECT id FROM groups WHERE remote_key = ?1",
            params![remote_key],
            |row| row.get(0),
        )
        .optional()?
    {
        return Ok(existing_id);
    }

    let base_name = repo_name_from_remote_key(remote_key).unwrap_or(fallback_name);
    create_unique_group(tx, base_name, Some(remote_key))
}

fn create_unique_group(
    tx: &Transaction<'_>,
    base_name: &str,
    remote_key: Option<&str>,
) -> Result<String, StoreError> {
    let name = unique_group_name(tx, base_name)?;
    let id = unique_id(tx, "groups")?;
    tx.execute(
        "INSERT INTO groups (id, name, remote_key) VALUES (?1, ?2, ?3)",
        params![&id, &name, remote_key],
    )?;
    Ok(id)
}

fn unique_group_name(tx: &Transaction<'_>, base: &str) -> Result<String, StoreError> {
    let mut candidate = base.to_string();
    let mut suffix = 2u32;
    loop {
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM groups WHERE name = ?1",
                params![&candidate],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(candidate);
        }
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
}

fn delete_group_if_empty(tx: &Transaction<'_>, group_id: &str) -> Result<(), StoreError> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM projects WHERE group_id = ?1",
        params![group_id],
        |row| row.get(0),
    )?;
    if count == 0 {
        tx.execute("DELETE FROM groups WHERE id = ?1", params![group_id])?;
    }
    Ok(())
}

fn unique_id(tx: &Transaction<'_>, table: &str) -> Result<String, StoreError> {
    let sql = match table {
        "groups" => "SELECT 1 FROM groups WHERE id = ?1",
        "projects" => "SELECT 1 FROM projects WHERE id = ?1",
        "accounts" => "SELECT 1 FROM accounts WHERE id = ?1",
        "worktrees" => "SELECT 1 FROM worktrees WHERE id = ?1",
        _ => unreachable!("tabla desconocida: {table}"),
    };
    let mut bytes = [0u8; 4];
    loop {
        getrandom::fill(&mut bytes).expect("falló getrandom");
        let id = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        );
        let exists: Option<i32> = tx
            .query_row(sql, params![&id], |row| row.get(0))
            .optional()?;
        if exists.is_none() {
            return Ok(id);
        }
    }
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
