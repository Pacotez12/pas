use std::thread;

const DEFAULT_BODY: &str = "Abrí pas para responder.";
const MAX_BODY_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingNotice {
    pub session_id: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub action_label: String,
}

impl WaitingNotice {
    pub fn new(
        session_id: impl Into<String>,
        title: impl Into<String>,
        last_line: Option<&str>,
    ) -> Self {
        let session_id = session_id.into();
        let title = title.into();
        let summary = format!("{title} está esperando tu respuesta");
        let body = last_line
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| truncate_chars(s, MAX_BODY_CHARS))
            .unwrap_or_else(|| DEFAULT_BODY.to_string());

        Self {
            session_id,
            title,
            summary,
            body,
            action_label: "Abrir".to_string(),
        }
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub trait Notifier: Send + Sync {
    fn notify_waiting(&self, notice: WaitingNotice, on_open: Box<dyn FnOnce() + Send>);
}

#[derive(Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify_waiting(&self, _notice: WaitingNotice, _on_open: Box<dyn FnOnce() + Send>) {}
}

#[derive(Debug, Default)]
pub struct DesktopNotifier;

impl DesktopNotifier {
    pub fn new() -> Self {
        Self
    }
}

impl Notifier for DesktopNotifier {
    fn notify_waiting(&self, notice: WaitingNotice, on_open: Box<dyn FnOnce() + Send>) {
        let _ = thread::Builder::new()
            .name("pas-notify".into())
            .spawn(move || {
                let result = notify_rust::Notification::new()
                    .appname("pas")
                    .summary(&notice.summary)
                    .body(&notice.body)
                    .action("open", &notice.action_label)
                    .show();

                match result {
                    Ok(handle) => {
                        handle.wait_for_action(|action| {
                            if action == "open" {
                                on_open();
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!("no se pudo mostrar el aviso de escritorio: {err}");
                    }
                }
            });
    }
}
