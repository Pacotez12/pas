use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use pas_core::socket::socket_path;
use pasd::config::Config;
use pasd::manager::Manager;
use pasd::notify::DesktopNotifier;
use pasd::quota::CODEX_POLL_INTERVAL;
use pasd::server;
use pasd::store::Store;
use tokio::signal::unix::{SignalKind, signal};

#[derive(Parser)]
#[command(name = "pasd", version)]
struct Args {
    #[arg(long)]
    socket: Option<PathBuf>,

    #[arg(long)]
    data_dir: Option<PathBuf>,

    #[arg(long, default_value_t = 1500)]
    idle_ms: u64,
}

fn default_data_dir() -> Result<PathBuf, String> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("pas"));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home).join(".local").join("share").join("pas"));
    }
    Err(
        "No se encontró HOME ni XDG_DATA_HOME. Pasá --data-dir con la carpeta de datos.".into(),
    )
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let path = match args.socket {
        Some(p) => p,
        None => match socket_path() {
            Ok(p) => p,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        },
    };

    let data_dir = match args.data_dir {
        Some(d) => d,
        None => match default_data_dir() {
            Ok(d) => d,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        },
    };

    let store = match Store::open(&data_dir.join("pas.db")) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    let listener = match server::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    let config = match Config::load_default() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!("pasd escuchando en {}", path.display());
    let mgr = Manager::with_config(
        args.idle_ms,
        store,
        Some(data_dir.join("accounts")),
        Some(path.clone()),
        config,
        Arc::new(DesktopNotifier::new()),
    );
    let quota_poller = mgr.spawn_quota_poller(CODEX_POLL_INTERVAL);

    tokio::select! {
        _ = server::serve(listener, Arc::clone(&mgr)) => {}
        _ = shutdown_signal() => {
            tracing::info!("señal de apagado recibida, cerrando pasd");
        }
    }

    quota_poller.abort();

    mgr.kill_all();
    let _ = std::fs::remove_file(&path);
    ExitCode::SUCCESS
}

async fn shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).expect("no se pudo instalar handler SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
}
