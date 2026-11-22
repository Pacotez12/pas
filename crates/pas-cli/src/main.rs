mod hook;
mod statusline;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use pas_core::CoreError;
use pas_core::autostart;
use pas_core::protocol::{
    CreateParams, CreateResult, ERR_WAIT_TIMEOUT, IdParams, METHOD_SESSION_CREATE,
    METHOD_SESSION_KILL, METHOD_SESSION_LIST, METHOD_SESSION_READ, METHOD_SESSION_SEND,
    METHOD_SESSION_WAIT, ReadParams, ReadResult, SendParams, SendResult, SessionInfo, SessionState,
    WaitParams, WaitUntil, WorktreeRequest,
};
use pas_core::socket::socket_path;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "pas", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    New {
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        worktree: bool,
        #[arg(long, requires = "worktree")]
        branch: Option<String>,
        #[arg(long, requires = "worktree")]
        base: Option<String>,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Send {
        session: String,
        text: String,
        #[arg(long)]
        no_enter: bool,
    },
    Read {
        session: String,
        #[arg(long)]
        lines: Option<usize>,
    },
    Wait {
        session: String,
        #[arg(long, value_enum, default_value_t = WaitUntilArg::Idle)]
        until: WaitUntilArg,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    Kill {
        session: String,
    },
    Hook {
        agent: String,
        event: String,
    },
    Statusline {
        #[arg(long)]
        agent: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum WaitUntilArg {
    Idle,
    Exited,
}

impl From<WaitUntilArg> for WaitUntil {
    fn from(value: WaitUntilArg) -> Self {
        match value {
            WaitUntilArg::Idle => WaitUntil::Idle,
            WaitUntilArg::Exited => WaitUntil::Exited,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Commands::Hook { agent, event } = cli.command {
        hook::run(&agent, &event).await;
        return ExitCode::SUCCESS;
    }
    if let Commands::Statusline { agent, command } = cli.command {
        return statusline::run(&agent, &command).await;
    }
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(CoreError::Rpc {
            code: ERR_WAIT_TIMEOUT,
            message,
        }) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<(), CoreError> {
    let path = socket_path()?;
    let mut client = autostart::connect_or_spawn(&path).await?;

    match cli.command {
        Commands::New {
            cwd,
            title,
            worktree,
            branch,
            base,
            command,
        } => {
            // pasd runs in its own folder, so a relative --cwd must be resolved here.
            let cwd = match cwd {
                Some(dir) => std::path::absolute(dir)?,
                None => std::env::current_dir()?,
            };
            let worktree = if worktree {
                Some(WorktreeRequest { branch, base })
            } else {
                None
            };
            let res: CreateResult = client
                .call(
                    METHOD_SESSION_CREATE,
                    CreateParams {
                        command,
                        cwd,
                        title,
                        env: BTreeMap::new(),
                        cols: 120,
                        rows: 40,
                        project_id: None,
                        account_id: None,
                        mode: pas_core::protocol::Mode::Terminal,
                        worktree,
                    },
                )
                .await?;
            println!("{}", res.id);
        }
        Commands::List { json: as_json } => {
            let list: Vec<SessionInfo> = client.call(METHOD_SESSION_LIST, json!({})).await?;
            if as_json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                print_table(&list);
            }
        }
        Commands::Send {
            session,
            text,
            no_enter,
        } => {
            let res: SendResult = client
                .call(
                    METHOD_SESSION_SEND,
                    SendParams {
                        id: session,
                        text,
                        enter: !no_enter,
                    },
                )
                .await?;
            if let Some(warning) = res.warning {
                eprintln!("{warning}");
            }
        }
        Commands::Read { session, lines } => {
            let res: ReadResult = client
                .call(METHOD_SESSION_READ, ReadParams { id: session, lines })
                .await?;
            if !res.text.is_empty() {
                println!("{}", res.text);
            }
        }
        Commands::Wait {
            session,
            until,
            timeout,
        } => {
            let st: SessionState = client
                .call(
                    METHOD_SESSION_WAIT,
                    WaitParams {
                        id: session,
                        until: until.into(),
                        timeout_ms: timeout.saturating_mul(1000),
                    },
                )
                .await?;
            println!("{}", format_state(&st));
        }
        Commands::Kill { session } => {
            let _: Value = client
                .call(METHOD_SESSION_KILL, IdParams { id: session })
                .await?;
        }
        Commands::Hook { .. } | Commands::Statusline { .. } => {
            unreachable!("hook y statusline se manejan antes de conectar a pasd")
        }
    }

    Ok(())
}

fn format_state(state: &SessionState) -> String {
    match state {
        SessionState::Running => "running".into(),
        SessionState::Waiting => "waiting".into(),
        SessionState::Idle => "idle".into(),
        SessionState::Exited { code: Some(c) } => format!("exited({c})"),
        SessionState::Exited { code: None } => "exited".into(),
    }
}

fn print_table(list: &[SessionInfo]) {
    let rows: Vec<(String, String, String, String)> = list
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                s.title.clone(),
                format_state(&s.state),
                s.cwd.display().to_string(),
            )
        })
        .collect();

    let id_w = rows.iter().map(|r| r.0.len()).max().unwrap_or(0).max(2);
    let title_w = rows.iter().map(|r| r.1.len()).max().unwrap_or(0).max(6);
    let state_w = rows.iter().map(|r| r.2.len()).max().unwrap_or(0).max(6);

    println!(
        "{:<id_w$}  {:<title_w$}  {:<state_w$}  CWD",
        "ID", "TÍTULO", "ESTADO"
    );
    for (id, title, state, cwd) in rows {
        println!("{id:<id_w$}  {title:<title_w$}  {state:<state_w$}  {cwd}");
    }
}
