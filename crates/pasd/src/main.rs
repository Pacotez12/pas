use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use pas_core::socket::socket_path;
use pasd::manager::Manager;
use pasd::server;
use tokio::signal::unix::{SignalKind, signal};

#[derive(Parser)]
#[command(name = "pasd", version)]
struct Args {
    #[arg(long)]
    socket: Option<PathBuf>,

    #[arg(long, default_value_t = 1500)]
    idle_ms: u64,
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

    let listener = match server::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!("pasd escuchando en {}", path.display());
    let mgr = Manager::new(args.idle_ms);

    tokio::select! {
        _ = server::serve(listener, Arc::clone(&mgr)) => {}
        _ = shutdown_signal() => {
            tracing::info!("señal de apagado recibida, cerrando pasd");
        }
    }

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
