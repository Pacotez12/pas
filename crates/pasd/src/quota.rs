use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pas_core::protocol::{
    AgentKind, Quota, QuotaChangedEvent, QuotaWarningEvent, QuotaWindow, QuotaWindowKind,
};
use serde_json::Value;

pub type Window = QuotaWindow;

pub const DEFAULT_WARN_PERCENT: f32 = 80.0;
pub const CODEX_POLL_INTERVAL: Duration = Duration::from_secs(60);

const WINDOW_JITTER_SECS: i64 = 120;
const CODEX_FIVE_HOUR_MINUTES: i64 = 300;
const CODEX_WEEK_MINUTES: i64 = 10080;
const MAX_RECENT_ROLLOUTS: usize = 3;
const ROLLOUT_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone)]
struct CachedRollout {
    mtime: SystemTime,
    len: u64,
    quota: Option<Quota>,
}

#[derive(Debug, Default)]
pub struct CodexScanCache {
    entries: HashMap<PathBuf, CachedRollout>,
}

#[derive(Debug)]
pub struct CodexScanResult {
    pub quota: Option<Quota>,
    pub files_read: usize,
}

#[derive(Debug, Default)]
pub struct QuotaUpdate {
    pub changed: Option<QuotaChangedEvent>,
    pub warnings: Vec<QuotaWarningEvent>,
}

#[derive(Debug)]
struct QuotaState {
    warn_percent: f32,
    by_account: HashMap<String, Quota>,
    warned: HashMap<(String, QuotaWindowKind), i64>,
}

#[derive(Debug)]
pub struct QuotaTracker {
    inner: RwLock<QuotaState>,
    codex_caches: Mutex<HashMap<String, CodexScanCache>>,
}

impl Default for QuotaTracker {
    fn default() -> Self {
        Self::new(DEFAULT_WARN_PERCENT)
    }
}

impl QuotaTracker {
    pub fn new(warn_percent: f32) -> Self {
        Self {
            inner: RwLock::new(QuotaState {
                warn_percent,
                by_account: HashMap::new(),
                warned: HashMap::new(),
            }),
            codex_caches: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_warn_percent(&self, percent: f32) {
        let mut state = self.inner.write().expect("quota lock poisoned");
        state.warn_percent = percent;
    }

    pub fn get(&self, account_id: &str) -> Option<Quota> {
        let state = self.inner.read().expect("quota lock poisoned");
        state.by_account.get(account_id).cloned()
    }

    pub fn remove(&self, account_id: &str) {
        let mut state = self.inner.write().expect("quota lock poisoned");
        state.by_account.remove(account_id);
        state.warned.retain(|(id, _), _| id != account_id);
        drop(state);
        self.codex_caches
            .lock()
            .expect("codex cache lock poisoned")
            .remove(account_id);
    }

    pub fn poll_codex_account(&self, account_id: &str, codex_home: &Path, now: i64) -> QuotaUpdate {
        let parsed = {
            let mut caches = self.codex_caches.lock().expect("codex cache lock poisoned");
            let cache = caches.entry(account_id.to_string()).or_default();
            read_codex_quota_cached(codex_home, cache, now).quota
        };
        match parsed {
            Some(quota) => self.record(account_id, quota),
            None => QuotaUpdate::default(),
        }
    }

    pub fn record(&self, account_id: &str, quota: Quota) -> QuotaUpdate {
        let mut state = self.inner.write().expect("quota lock poisoned");

        if let Some(prev) = state.by_account.get(account_id)
            && prev.five_h == quota.five_h
            && prev.week == quota.week
        {
            return QuotaUpdate::default();
        }

        state
            .by_account
            .insert(account_id.to_string(), quota.clone());

        let warn_percent = state.warn_percent;
        let mut warnings = Vec::new();
        for (kind, maybe_win) in [
            (QuotaWindowKind::FiveH, quota.five_h.as_ref()),
            (QuotaWindowKind::Week, quota.week.as_ref()),
        ] {
            let Some(win) = maybe_win else {
                continue;
            };
            let key = (account_id.to_string(), kind);
            if win.used_percent >= warn_percent {
                let already_warned = state.warned.get(&key).is_some_and(|&prev_reset| {
                    (win.resets_at - prev_reset).abs() <= WINDOW_JITTER_SECS
                });
                if !already_warned {
                    state.warned.insert(key, win.resets_at);
                    warnings.push(QuotaWarningEvent {
                        account_id: account_id.to_string(),
                        window: kind,
                        used_percent: win.used_percent,
                        resets_at: win.resets_at,
                        threshold_percent: warn_percent,
                    });
                }
            } else if win.resets_at == 0 {
                state.warned.remove(&key);
            }
        }

        QuotaUpdate {
            changed: Some(QuotaChangedEvent {
                account_id: account_id.to_string(),
                quota,
            }),
            warnings,
        }
    }
}

pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn parse_report(agent: AgentKind, payload: &Value, now: i64) -> Option<Quota> {
    match agent {
        AgentKind::Claude => parse_claude_report(payload, now),
        AgentKind::Agy => parse_agy_report(payload, now),
        AgentKind::Codex => None,
    }
}

pub fn parse_claude_report(payload: &Value, now: i64) -> Option<Quota> {
    let rl = payload.get("rate_limits")?.as_object()?;
    let five_h = rl.get("five_hour").and_then(parse_claude_window);
    let week = rl.get("seven_day").and_then(parse_claude_window);
    if five_h.is_none() && week.is_none() {
        return None;
    }
    Some(Quota {
        five_h,
        week,
        updated_at: now,
    })
}

fn parse_claude_window(val: &Value) -> Option<QuotaWindow> {
    let obj = val.as_object()?;
    let used_percent = json_f32(obj.get("used_percentage")?)?;
    let resets_at = obj.get("resets_at").and_then(json_i64).unwrap_or(0);
    Some(QuotaWindow {
        used_percent,
        resets_at,
    })
}

pub fn parse_agy_report(payload: &Value, now: i64) -> Option<Quota> {
    let quota_obj = payload.get("quota")?.as_object()?;
    let model_name = payload
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(Value::as_str);

    let prefer_gemini = model_name
        .map(|name| name.to_ascii_lowercase().contains("gemini"))
        .unwrap_or(true);

    let groups = if prefer_gemini {
        [("gemini-5h", "gemini-weekly"), ("3p-5h", "3p-weekly")]
    } else {
        [("3p-5h", "3p-weekly"), ("gemini-5h", "gemini-weekly")]
    };

    for (five_key, week_key) in groups {
        let five_h = quota_obj
            .get(five_key)
            .and_then(|v| parse_agy_window(v, now));
        let week = quota_obj
            .get(week_key)
            .and_then(|v| parse_agy_window(v, now));
        if five_h.is_some() || week.is_some() {
            return Some(Quota {
                five_h,
                week,
                updated_at: now,
            });
        }
    }

    None
}

fn parse_agy_window(val: &Value, now: i64) -> Option<QuotaWindow> {
    let obj = val.as_object()?;
    let remaining = json_f64(obj.get("remaining_fraction")?)?;
    let used_percent = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0) as f32;
    let resets_at = obj
        .get("reset_in_seconds")
        .and_then(json_f64)
        .filter(|&s| s >= 0.0)
        .map(|s| now + s.round() as i64)
        .unwrap_or(0);
    Some(QuotaWindow {
        used_percent,
        resets_at,
    })
}

pub fn read_codex_quota(codex_home: &Path, now: i64) -> Option<Quota> {
    read_codex_quota_cached(codex_home, &mut CodexScanCache::default(), now).quota
}

pub fn read_codex_quota_cached(
    codex_home: &Path,
    cache: &mut CodexScanCache,
    now: i64,
) -> CodexScanResult {
    let sessions_dir = codex_home.join("sessions");
    if !sessions_dir.is_dir() {
        cache.entries.clear();
        return CodexScanResult {
            quota: None,
            files_read: 0,
        };
    }

    let recent = find_recent_rollouts(&sessions_dir, MAX_RECENT_ROLLOUTS);
    cache
        .entries
        .retain(|path, _| recent.iter().any(|m| &m.path == path));

    let mut files_read = 0;
    for meta in recent {
        if let Some(cached) = cache.entries.get(&meta.path)
            && cached.mtime == meta.mtime
            && cached.len == meta.len
        {
            if let Some(q) = &cached.quota {
                return CodexScanResult {
                    quota: Some(q.clone()),
                    files_read,
                };
            }
            continue;
        }

        files_read += 1;
        let parsed = read_tail(&meta.path, ROLLOUT_TAIL_BYTES)
            .ok()
            .and_then(|tail| parse_codex_rollout_str(&tail, now));
        cache.entries.insert(
            meta.path,
            CachedRollout {
                mtime: meta.mtime,
                len: meta.len,
                quota: parsed.clone(),
            },
        );
        if parsed.is_some() {
            return CodexScanResult {
                quota: parsed,
                files_read,
            };
        }
    }

    CodexScanResult {
        quota: None,
        files_read,
    }
}

struct RolloutMeta {
    path: PathBuf,
    mtime: SystemTime,
    len: u64,
}

fn find_recent_rollouts(sessions_dir: &Path, limit: usize) -> Vec<RolloutMeta> {
    let mut found = Vec::new();
    collect_recent_jsonl(sessions_dir, limit, &mut found);
    found.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| b.path.cmp(&a.path)));
    found.truncate(limit);
    found
}

fn collect_recent_jsonl(dir: &Path, limit: usize, out: &mut Vec<RolloutMeta>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<(std::ffi::OsString, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            subdirs.push((entry.file_name(), path));
        } else if meta.is_file() && path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            out.push(RolloutMeta {
                path,
                mtime,
                len: meta.len(),
            });
        }
    }

    if out.len() >= limit {
        return;
    }

    subdirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, subdir) in subdirs {
        if out.len() >= limit {
            break;
        }
        collect_recent_jsonl(&subdir, limit, out);
    }
}

fn read_tail(path: &Path, max_bytes: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut buf = Vec::with_capacity(len.min(max_bytes) as usize);
    let truncated = len > max_bytes;
    if truncated {
        file.seek(SeekFrom::End(-(max_bytes as i64)))?;
    }
    file.read_to_end(&mut buf)?;

    let slice = if truncated {
        match buf.iter().position(|&b| b == b'\n') {
            Some(pos) => &buf[pos + 1..],
            None => &buf[..],
        }
    } else {
        &buf[..]
    };
    Ok(String::from_utf8_lossy(slice).into_owned())
}

pub fn parse_codex_rollout_str(content: &str, now: i64) -> Option<Quota> {
    for line in content.lines().rev() {
        if !line.contains("rate_limits") {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(rl) = val
            .get("rate_limits")
            .or_else(|| val.get("payload").and_then(|p| p.get("rate_limits")))
            .and_then(Value::as_object)
        else {
            continue;
        };

        let mut five_h = None;
        let mut week = None;

        for (is_primary, key) in [(true, "primary"), (false, "secondary")] {
            let Some((window_minutes, window)) = rl.get(key).and_then(parse_codex_bucket) else {
                continue;
            };
            match window_minutes {
                Some(CODEX_FIVE_HOUR_MINUTES) => five_h = Some(window),
                Some(CODEX_WEEK_MINUTES) => week = Some(window),
                _ if is_primary => five_h = Some(window),
                _ => week = Some(window),
            }
        }

        if five_h.is_some() || week.is_some() {
            return Some(Quota {
                five_h,
                week,
                updated_at: now,
            });
        }
    }

    None
}

fn parse_codex_bucket(val: &Value) -> Option<(Option<i64>, QuotaWindow)> {
    let obj = val.as_object()?;
    let used_percent = json_f32(obj.get("used_percent")?)?;
    let resets_at = obj.get("resets_at").and_then(json_i64).unwrap_or(0);
    let window_minutes = obj.get("window_minutes").and_then(json_i64);
    Some((
        window_minutes,
        QuotaWindow {
            used_percent,
            resets_at,
        },
    ))
}

fn json_f64(val: &Value) -> Option<f64> {
    val.as_f64().filter(|v| v.is_finite())
}

fn json_f32(val: &Value) -> Option<f32> {
    json_f64(val).map(|v| v as f32)
}

fn json_i64(val: &Value) -> Option<i64> {
    val.as_i64().or_else(|| {
        val.as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v.round() as i64)
    })
}
