use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pas_core::protocol::{
    ERR_INVALID_PARAMS, ERR_WORKTREE_BRANCH_EXISTS, ERR_WORKTREE_CONFIRM_REQUIRED,
    ERR_WORKTREE_DIRTY_MAIN, ERR_WORKTREE_GIT_FAILED, ERR_WORKTREE_MERGE_CONFLICT,
    ERR_WORKTREE_WRONG_BASE, RpcError, WorktreeCreateParams, WorktreeInfo, WorktreeListItem,
    WorktreeState, WorktreeStatus, WorktreeSuggestBranchParams, WorktreeSuggestBranchResult,
};

use crate::rpc::store_error_to_rpc;
use crate::store::{Store, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(
        "No hay una carpeta configurada para crear worktrees. Configurá `worktrees_dir` o pasá `--data-dir`."
    )]
    NoWorktreesDir,

    #[error(
        "El repositorio está en HEAD desacoplado. Cambiá a una rama o indicá una rama base en \"Desde\"."
    )]
    DetachedHead,

    #[error("La rama `{branch}` ya existe. Elegí otro nombre de rama.")]
    BranchAlreadyExists { branch: String },

    #[error(
        "El nombre de rama `{branch}` no es válido para git. Usá letras, números, guiones o barras."
    )]
    InvalidBranchName { branch: String },

    #[error("No existe la rama base `{base}` en el repositorio. Elegí una rama existente.")]
    BaseNotFound { base: String },

    #[error(
        "No se puede mergear {branch}: el repo principal está en la rama {current} en vez de {expected}. Cambiá a {expected} y volvé a intentar."
    )]
    WrongBaseBranch {
        branch: String,
        current: String,
        expected: String,
    },

    #[error(
        "No se puede mergear {branch}: el repo principal tiene cambios sin commitear. Commiteá o guardá esos cambios primero."
    )]
    DirtyMainRepo { branch: String },

    #[error(
        "No se pudo mergear {branch} porque hay conflictos en: {files}. Se canceló el merge sin tocar el repo; resolvé el conflicto a mano."
    )]
    MergeConflict { branch: String, files: String },

    #[error("{message}")]
    ConfirmLossRequired { message: String },

    #[error("Falló git {action}: {detail}. Revisá el estado del repositorio e intentá de nuevo.")]
    GitFailed {
        action: &'static str,
        detail: String,
    },

    #[error("No se pudo acceder a la carpeta del worktree: {0}. Revisá los permisos.")]
    Io(#[from] std::io::Error),
}

pub fn worktree_error_to_rpc(err: WorktreeError) -> RpcError {
    match err {
        WorktreeError::Store(e) => store_error_to_rpc(e),
        other => {
            let code = match other {
                WorktreeError::Store(_) => unreachable!(),
                WorktreeError::NoWorktreesDir
                | WorktreeError::DetachedHead
                | WorktreeError::InvalidBranchName { .. }
                | WorktreeError::BaseNotFound { .. } => ERR_INVALID_PARAMS,
                WorktreeError::BranchAlreadyExists { .. } => ERR_WORKTREE_BRANCH_EXISTS,
                WorktreeError::WrongBaseBranch { .. } => ERR_WORKTREE_WRONG_BASE,
                WorktreeError::DirtyMainRepo { .. } => ERR_WORKTREE_DIRTY_MAIN,
                WorktreeError::MergeConflict { .. } => ERR_WORKTREE_MERGE_CONFLICT,
                WorktreeError::ConfirmLossRequired { .. } => ERR_WORKTREE_CONFIRM_REQUIRED,
                WorktreeError::GitFailed { .. } => ERR_WORKTREE_GIT_FAILED,
                WorktreeError::Io(_) => -32603,
            };
            RpcError {
                code,
                message: other.to_string(),
            }
        }
    }
}

pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "~" {
        if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty())
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

pub fn git_repo_root(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let trimmed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

pub fn suggest_branch(
    store: &Store,
    worktrees_dir: Option<&Path>,
    params: WorktreeSuggestBranchParams,
) -> Result<WorktreeSuggestBranchResult, WorktreeError> {
    let worktrees_dir = worktrees_dir.ok_or(WorktreeError::NoWorktreesDir)?;
    let project = store.get_project(&params.project_id)?;
    let base = current_branch(&project.path)?;
    let prefix = build_branch_proposal(params.agent.as_deref(), params.title.as_deref());
    let (branch, path) =
        pick_unique_branch_and_path(&project.path, worktrees_dir, &project.name, &prefix);
    Ok(WorktreeSuggestBranchResult { branch, base, path })
}

pub fn create(
    store: &Store,
    worktrees_dir: Option<&Path>,
    params: WorktreeCreateParams,
) -> Result<WorktreeInfo, WorktreeError> {
    let worktrees_dir = worktrees_dir.ok_or(WorktreeError::NoWorktreesDir)?;
    let project = store.get_project(&params.project_id)?;

    let base = match params
        .base
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(b) => {
            if !ref_exists(&project.path, b) {
                return Err(WorktreeError::BaseNotFound {
                    base: b.to_string(),
                });
            }
            b.to_string()
        }
        None => current_branch(&project.path)?,
    };

    let (branch, target_path) = match params
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(explicit) => {
            if !is_valid_branch_name(&project.path, explicit) {
                return Err(WorktreeError::InvalidBranchName {
                    branch: explicit.to_string(),
                });
            }
            if local_branch_exists(&project.path, explicit) {
                return Err(WorktreeError::BranchAlreadyExists {
                    branch: explicit.to_string(),
                });
            }
            let path = worktree_path(worktrees_dir, &project.name, explicit);
            (explicit.to_string(), path)
        }
        None => {
            let proposal = build_branch_proposal(params.agent.as_deref(), params.title.as_deref());
            pick_unique_branch_and_path(&project.path, worktrees_dir, &project.name, &proposal)
        }
    };

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    run_git(
        &project.path,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &target_path.to_string_lossy(),
            &base,
        ],
        "worktree add",
    )?;

    let info = store.insert_worktree(&project.id, &branch, &base, &target_path, None)?;
    Ok(info)
}

pub fn list(store: &Store, project_id: &str) -> Result<Vec<WorktreeListItem>, WorktreeError> {
    let project = store.get_project(project_id)?;
    let worktrees = store.list_worktrees(project_id)?;
    let mut items = Vec::with_capacity(worktrees.len());
    for info in worktrees {
        let status = inspect_status(&project.path, &info);
        items.push(WorktreeListItem { info, status });
    }
    Ok(items)
}

pub fn merge(store: &Store, id: &str) -> Result<WorktreeInfo, WorktreeError> {
    let wt = store.get_worktree(id)?;
    if wt.state == WorktreeState::Removed {
        return Err(WorktreeError::Store(StoreError::WorktreeNotFound));
    }
    let project = store.get_project(&wt.project_id)?;

    let current = current_branch(&project.path)?;
    if current != wt.base {
        return Err(WorktreeError::WrongBaseBranch {
            branch: wt.branch,
            current,
            expected: wt.base,
        });
    }

    let dirty = porcelain_files(&project.path);
    if !dirty.is_empty() {
        return Err(WorktreeError::DirtyMainRepo { branch: wt.branch });
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(&project.path)
        .args(["merge", "--no-edit", &wt.branch])
        .output()?;

    if !output.status.success() {
        let conflict_files = conflicted_files(&project.path);
        let _ = Command::new("git")
            .arg("-C")
            .arg(&project.path)
            .args(["merge", "--abort"])
            .output();

        if !conflict_files.is_empty() {
            return Err(WorktreeError::MergeConflict {
                branch: wt.branch,
                files: conflict_files.join(", "),
            });
        }
        let detail = git_error_detail(&output);
        return Err(WorktreeError::GitFailed {
            action: "merge",
            detail,
        });
    }

    let updated = store.set_worktree_state(&wt.id, WorktreeState::Merged)?;
    Ok(updated)
}

pub fn remove(store: &Store, id: &str, confirm_loss: bool) -> Result<(), WorktreeError> {
    let wt = store.get_worktree(id)?;
    if wt.state == WorktreeState::Removed {
        return Err(WorktreeError::Store(StoreError::WorktreeNotFound));
    }
    let project = store.get_project(&wt.project_id)?;
    let status = inspect_status(&project.path, &wt);

    let uncommitted_count = status.uncommitted.len();
    let has_uncommitted = uncommitted_count > 0;
    let has_unmerged = status.ahead > 0 && !status.merged;

    if !confirm_loss && (has_uncommitted || has_unmerged) {
        let message = format_loss_confirmation_message(
            &wt.branch,
            &wt.base,
            uncommitted_count,
            status.ahead,
            has_unmerged,
        );
        return Err(WorktreeError::ConfirmLossRequired { message });
    }

    if status.missing {
        run_git(&project.path, &["worktree", "prune"], "worktree prune")?;
    } else if confirm_loss {
        run_git(
            &project.path,
            &["worktree", "remove", "--force", &wt.path.to_string_lossy()],
            "worktree remove",
        )?;
    } else {
        run_git(
            &project.path,
            &["worktree", "remove", &wt.path.to_string_lossy()],
            "worktree remove",
        )?;
    }

    if local_branch_exists(&project.path, &wt.branch) {
        // Loss was already checked against the base above; `-d` would compare against
        // whatever the main repo has checked out and could fail after the folder is gone.
        run_git(&project.path, &["branch", "-D", &wt.branch], "branch")?;
    }

    store.set_worktree_state(&wt.id, WorktreeState::Removed)?;
    Ok(())
}

fn format_loss_confirmation_message(
    branch: &str,
    base: &str,
    uncommitted: usize,
    ahead: u32,
    has_unmerged: bool,
) -> String {
    let files_label = if uncommitted == 1 {
        "1 archivo sin commitear".to_string()
    } else {
        format!("{uncommitted} archivos sin commitear")
    };
    let commits_label = if ahead == 1 {
        "1 commit sin mergear".to_string()
    } else {
        format!("{ahead} commits sin mergear")
    };

    if uncommitted > 0 && has_unmerged {
        format!(
            "No se puede borrar el worktree {branch}: tiene {files_label} y {commits_label} en {base}. Marcá la casilla para confirmar que se pierden o commiteá y mergeá primero."
        )
    } else if uncommitted > 0 {
        format!(
            "No se puede borrar el worktree {branch}: tiene {files_label}. Marcá la casilla para confirmar que se pierden o commitealos primero."
        )
    } else {
        format!(
            "No se puede borrar el worktree {branch}: tiene {commits_label} en {base}. Marcá la casilla para confirmar que se pierden o mergeá la rama primero."
        )
    }
}

fn inspect_status(repo: &Path, wt: &WorktreeInfo) -> WorktreeStatus {
    let missing = !wt.path.exists();
    let uncommitted = if missing {
        Vec::new()
    } else {
        porcelain_files(&wt.path)
    };

    let branch_exists = local_branch_exists(repo, &wt.branch);
    let ahead = if branch_exists {
        commits_ahead(repo, &wt.base, &wt.branch)
    } else {
        0
    };
    let merged = if wt.state == WorktreeState::Merged {
        true
    } else if branch_exists {
        is_ancestor(repo, &wt.branch, &wt.base)
    } else {
        false
    };

    WorktreeStatus {
        uncommitted,
        ahead,
        merged,
        missing,
    }
}

fn build_branch_proposal(agent: Option<&str>, title: Option<&str>) -> String {
    let agent_slug = agent.map(slugify).filter(|s| !s.is_empty());
    let title_slug = title.map(slugify).filter(|s| !s.is_empty());
    match (agent_slug, title_slug) {
        (Some(a), Some(t)) => format!("pas/{a}-{t}"),
        (Some(a), None) => format!("pas/{a}"),
        (None, Some(t)) => format!("pas/{t}"),
        (None, None) => "pas/sesion".to_string(),
    }
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.trim().chars() {
        let mapped = match ch.to_ascii_lowercase() {
            'a'..='z' | '0'..='9' => Some(ch.to_ascii_lowercase()),
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => Some('a'),
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => Some('e'),
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => Some('i'),
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => Some('o'),
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => Some('u'),
            'ñ' | 'Ñ' => Some('n'),
            _ => None,
        };
        match mapped {
            Some(c) => {
                out.push(c);
                prev_dash = false;
            }
            None => {
                if !out.is_empty() && !prev_dash {
                    out.push('-');
                    prev_dash = true;
                }
            }
        }
        if out.len() >= 48 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn sanitize_dir_component(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '-',
            other => other,
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "proyecto".to_string()
    } else {
        cleaned
    }
}

fn worktree_path(worktrees_dir: &Path, project_name: &str, branch: &str) -> PathBuf {
    let project_dir = sanitize_dir_component(project_name);
    let branch_dir = branch.replace(['/', '\\'], "-");
    worktrees_dir.join(project_dir).join(branch_dir)
}

fn pick_unique_branch_and_path(
    repo: &Path,
    worktrees_dir: &Path,
    project_name: &str,
    base_proposal: &str,
) -> (String, PathBuf) {
    let mut candidate = base_proposal.to_string();
    let mut suffix = 2u32;
    loop {
        let path = worktree_path(worktrees_dir, project_name, &candidate);
        if !local_branch_exists(repo, &candidate) && !path.exists() {
            return (candidate, path);
        }
        candidate = format!("{base_proposal}-{suffix}");
        suffix += 1;
    }
}

fn current_branch(repo: &Path) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()?;
    if !output.status.success() {
        return Err(WorktreeError::DetachedHead);
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        return Err(WorktreeError::DetachedHead);
    }
    Ok(branch)
}

fn local_branch_exists(repo: &Path, branch: &str) -> bool {
    let ref_name = format!("refs/heads/{branch}");
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", &ref_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ref_exists(repo: &Path, rev: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", rev])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_valid_branch_name(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["check-ref-format", "--branch", branch])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn porcelain_files(dir: &Path) -> Vec<String> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    raw.trim_end_matches(['\r', '\n'])
        .lines()
        .filter(|line| line.len() > 3)
        .map(|line| {
            let entry = &line[3..];
            let entry = entry.split(" -> ").last().unwrap_or(entry);
            entry.trim_matches('"').to_string()
        })
        .collect()
}

fn conflicted_files(repo: &Path) -> Vec<String> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["diff", "--name-only", "--diff-filter=U"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn commits_ahead(repo: &Path, base: &str, branch: &str) -> u32 {
    let range = format!("{base}..{branch}");
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", &range])
        .output()
    else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .unwrap_or(0)
}

fn is_ancestor(repo: &Path, branch: &str, base: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", branch, base])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_git(repo: &Path, args: &[&str], action: &'static str) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(WorktreeError::GitFailed {
            action,
            detail: git_error_detail(&output),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_error_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    "código de salida distinto de cero".to_string()
}
