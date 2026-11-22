use std::path::Path;
use std::process::Command;

/// Normaliza una URL de remoto git para que ssh y https den la misma clave:
/// sin `.git` final, sin barras finales, host y ruta en minúsculas.
/// Ejemplo: `git@github.com:A/b.git` y `https://github.com/a/b` -> `Some("github.com/a/b")`.
pub fn remote_key(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (host, path) = if let Some((_scheme, rest)) = trimmed.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, hp)| hp);
        let host = strip_numeric_port(host_port);
        (host, path)
    } else if let Some((left, path)) = trimmed.split_once(':') {
        // Sintaxis SCP: [user@]host:path (no debe haber '/' antes de ':')
        if left.contains('/') {
            return None;
        }
        let host = left.rsplit_once('@').map_or(left, |(_, h)| h);
        (host, path)
    } else {
        return None;
    };

    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }

    let mut clean_path = path.trim().trim_matches('/');
    if let Some(stripped) = clean_path.strip_suffix(".git") {
        clean_path = stripped.trim_end_matches('/');
    } else if clean_path.len() >= 4
        && clean_path[clean_path.len() - 4..].eq_ignore_ascii_case(".git")
    {
        clean_path = clean_path[..clean_path.len() - 4].trim_end_matches('/');
    }

    if clean_path.is_empty() {
        return None;
    }

    Some(format!("{host}/{}", clean_path.to_ascii_lowercase()))
}

fn strip_numeric_port(host_port: &str) -> &str {
    if let Some((host, port)) = host_port.rsplit_once(':')
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
    {
        return host;
    }
    host_port
}

/// Lee el remoto `origin` de un repositorio git y devuelve su `remote_key` normalizada.
pub fn read_origin_remote_key(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout);
    remote_key(&url)
}

/// Extrae el nombre del repo (último segmento) de una `remote_key` normalizada.
pub fn repo_name_from_remote_key(key: &str) -> Option<&str> {
    key.rsplit('/').find(|seg| !seg.is_empty())
}
