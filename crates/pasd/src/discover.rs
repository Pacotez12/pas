use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pas_core::protocol::{AgentKind, DiscoveredAccount, DiscoveredProject};

use crate::open::expand_tilde_with_home;
use crate::remote::read_origin_remote_key;
use crate::store::{Store, StoreError};

pub const DEFAULT_MAX_RESULTS: usize = 200;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_MAX_DEPTH: usize = 3;

const SKIP_DIR_NAMES: &[&str] = &[".git", "node_modules", "target", "build"];

pub fn discover_projects(
    store: &Store,
    discover_dirs: Option<&[PathBuf]>,
    home_dir: Option<&Path>,
) -> Result<Vec<DiscoveredProject>, StoreError> {
    discover_projects_with_limits(
        store,
        discover_dirs,
        home_dir,
        DEFAULT_MAX_RESULTS,
        DEFAULT_TIMEOUT,
    )
}

pub fn discover_projects_with_limits(
    store: &Store,
    discover_dirs: Option<&[PathBuf]>,
    home_dir: Option<&Path>,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<DiscoveredProject>, StoreError> {
    let started = Instant::now();
    let home = resolve_home(home_dir);

    let roots: Vec<PathBuf> = match discover_dirs {
        Some(dirs) => dirs
            .iter()
            .map(|d| expand_tilde_with_home(d, home.as_deref()))
            .collect(),
        None => match home.as_deref() {
            Some(h) => vec![
                h.join("Proyectos"),
                h.join("orca").join("projects"),
                h.join("orca").join("workspaces"),
                h.join("Trabajo"),
            ],
            None => Vec::new(),
        },
    };

    let added: HashSet<PathBuf> = store
        .list_projects()?
        .into_iter()
        .map(|p| fs::canonicalize(&p.path).unwrap_or(p.path))
        .collect();

    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    for root in roots {
        if root.is_dir() {
            queue.push_back((root, 0));
        }
    }

    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut found_paths: HashSet<PathBuf> = HashSet::new();
    let mut results: Vec<DiscoveredProject> = Vec::new();

    while let Some((dir, depth)) = queue.pop_front() {
        if results.len() >= max_results || started.elapsed() >= timeout {
            break;
        }
        let Ok(canon) = fs::canonicalize(&dir) else {
            continue;
        };
        if !visited.insert(canon.clone()) {
            continue;
        }

        if canon.join(".git").exists() {
            if !added.contains(&canon) && found_paths.insert(canon.clone()) {
                let name = canon
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "proyecto".into());
                let remote_key = read_origin_remote_key(&canon);
                results.push(DiscoveredProject {
                    name,
                    path: canon,
                    remote_key,
                });
            }
            continue;
        }

        if depth >= DEFAULT_MAX_DEPTH {
            continue;
        }

        let Ok(entries) = fs::read_dir(&canon) else {
            continue;
        };
        for entry in entries.flatten() {
            if results.len() >= max_results || started.elapsed() >= timeout {
                break;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            if let Some(name) = file_name.to_str()
                && SKIP_DIR_NAMES.contains(&name)
            {
                continue;
            }
            queue.push_back((entry.path(), depth + 1));
        }
    }

    results.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(results)
}

pub fn discover_accounts(
    store: &Store,
    home_dir: Option<&Path>,
) -> Result<Vec<DiscoveredAccount>, StoreError> {
    let Some(home) = resolve_home(home_dir) else {
        return Ok(Vec::new());
    };

    let existing: HashMap<PathBuf, String> = store
        .list_accounts()?
        .into_iter()
        .map(|acc| {
            let canon = fs::canonicalize(&acc.config_dir).unwrap_or(acc.config_dir);
            (canon, acc.name)
        })
        .collect();

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut results: Vec<DiscoveredAccount> = Vec::new();

    let mut push_candidate = |agent: AgentKind, suggested_name: String, dir: PathBuf| {
        if !dir.is_dir() {
            return;
        }
        let Ok(canon) = fs::canonicalize(&dir) else {
            return;
        };
        if !seen.insert(canon.clone()) {
            return;
        }
        let already_added_as = existing.get(&canon).cloned();
        results.push(DiscoveredAccount {
            agent,
            suggested_name,
            config_dir: canon,
            already_added_as,
        });
    };

    let orca_profiles = home.join(".config").join("orca").join("claude-profiles");
    if let Ok(entries) = fs::read_dir(&orca_profiles) {
        let mut profiles: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                let home_subdir = entry.path().join("home");
                home_subdir.is_dir().then_some((name, home_subdir))
            })
            .collect();
        profiles.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, dir) in profiles {
            push_candidate(AgentKind::Claude, name, dir);
        }
    }

    push_candidate(AgentKind::Claude, "Claude".into(), home.join(".claude"));
    push_candidate(AgentKind::Codex, "Codex".into(), home.join(".codex"));

    Ok(results)
}

fn resolve_home(home_dir: Option<&Path>) -> Option<PathBuf> {
    home_dir.map(Path::to_path_buf).or_else(|| {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
}
