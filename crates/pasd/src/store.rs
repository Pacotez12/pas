use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use pas_core::protocol::{GroupInfo, ProjectInfo};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::remote::{read_origin_remote_key, repo_name_from_remote_key};

const SCHEMA_VERSION: i32 = 1;

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
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        init_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        init_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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
            .query_row("SELECT id FROM groups WHERE name = ?1", params![name], |row| {
                row.get(0)
            })
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
}

fn init_connection(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < SCHEMA_VERSION {
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
    Ok(())
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
        _ => unreachable!("tabla desconocida: {table}"),
    };
    let mut bytes = [0u8; 4];
    loop {
        getrandom::fill(&mut bytes).expect("falló getrandom");
        let id = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        );
        let exists: Option<i32> = tx.query_row(sql, params![&id], |row| row.get(0)).optional()?;
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
