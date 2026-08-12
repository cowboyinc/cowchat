use clap::{Parser, Subcommand};
use cowchat_server::store::Store;
use cowchat_server::{auth, lock_database_for_maintenance, CowchatServer, ServerConfig};
use std::{
    io::{self, BufRead},
    path::PathBuf,
    thread,
    time::Duration,
};

const APP_CONTROL_EOF_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppControlCommand {
    Interrupt,
    Terminate,
    Kill,
}

impl AppControlCommand {
    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "INT" => Some(Self::Interrupt),
            "TERM" => Some(Self::Terminate),
            "KILL" => Some(Self::Kill),
            _ => None,
        }
    }
}

#[derive(Parser)]
#[command(name = "cowchat-server", version, about = "Cowchat server daemon")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Cowchat server
    Serve {
        /// Unix socket path
        #[arg(long, default_value = default_socket_path())]
        socket: PathBuf,

        /// TCP bind address (set to empty or use --no-tcp to disable)
        #[arg(long, default_value = "127.0.0.1:9229")]
        tcp: String,

        /// Disable TCP listener
        #[arg(long)]
        no_tcp: bool,

        /// HTTP/WebSocket bind address (e.g., 0.0.0.0:8080)
        #[arg(long)]
        http: Option<String>,

        /// Allow POST /api/keys (also requires --http-admin-secret).
        #[arg(long, requires = "http_admin_secret")]
        enable_http_signup: bool,

        /// Secret required in X-Cowchat-Admin for HTTP key creation
        ///.
        #[arg(long)]
        http_admin_secret: Option<String>,

        /// Browser Origin allowed to use the HTTP/WebSocket surface. Repeatable.
        #[arg(long = "http-origin")]
        http_origins: Vec<String>,

        /// Proxy IP allowed to supply the final X-Forwarded-For hop. Repeatable.
        #[arg(long = "trusted-proxy")]
        trusted_proxy_ips: Vec<std::net::IpAddr>,

        /// Disable API key validation (open access, for local dev)
        #[arg(long)]
        no_auth: bool,

        /// Require API keys even over the Unix socket and loopback TCP
        #[arg(long)]
        require_local_auth: bool,

        /// SQLite database path
        #[arg(long, default_value = default_db_path())]
        db: PathBuf,

        /// API key file path
        #[arg(long, default_value = default_key_path())]
        key_file: PathBuf,

        /// Accept lifecycle commands from the Cowchat app over inherited stdin.
        #[arg(long, hide = true)]
        app_control_stdin: bool,
    },

    /// Manage authentication
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Show the current API key
    ShowKey {
        #[arg(long, default_value = default_key_path())]
        key_file: PathBuf,
    },
    /// Rotate the API key (generates a new one)
    RotateKey {
        #[arg(long, default_value = default_key_path())]
        key_file: PathBuf,
        /// SQLite database whose ownership records must survive the rotation.
        #[arg(long, default_value = default_db_path())]
        db: PathBuf,
    },
    /// Recover ownership after a legacy rotation by mapping the previous key
    /// onto the stable primary credential principal. The server must be
    /// stopped, and the previous key is required to avoid taking another
    /// credential's resources.
    MigratePrimaryOwnership {
        /// Owner-only file containing the previous primary key. Keeping the
        /// secret out of argv avoids shell-history and process-list exposure.
        #[arg(long)]
        previous_key_file: PathBuf,
        #[arg(long, default_value = default_db_path())]
        db: PathBuf,
    },
}

fn default_data_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".cowchat"))
        .unwrap_or_else(|| PathBuf::from(".cowchat"))
}

fn default_socket_path() -> &'static str {
    // Leak the string to get a 'static str for clap default
    Box::leak(
        default_data_dir()
            .join("cowchat.sock")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    )
}

fn default_db_path() -> &'static str {
    Box::leak(
        default_data_dir()
            .join("cowchat.db")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    )
}

fn default_key_path() -> &'static str {
    Box::leak(
        default_data_dir()
            .join("auth.key")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    )
}

fn monitor_app_control<R, D, W>(mut reader: R, mut dispatch: D, mut wait: W) -> io::Result<()>
where
    R: BufRead,
    D: FnMut(AppControlCommand),
    W: FnMut(Duration),
{
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // If the owning app disappears without an orderly handshake,
                // stdin reaches EOF. Give the server one bounded TERM window,
                // then make it impossible for the helper to remain orphaned.
                dispatch(AppControlCommand::Terminate);
                wait(APP_CONTROL_EOF_GRACE);
                dispatch(AppControlCommand::Kill);
                return Ok(());
            }
            Ok(_) => {
                let Some(command) = AppControlCommand::parse(&line) else {
                    continue;
                };
                dispatch(command);
                if command == AppControlCommand::Kill {
                    return Ok(());
                }
            }
            Err(error) => {
                // A broken ownership channel has the same meaning as EOF.
                dispatch(AppControlCommand::Terminate);
                wait(APP_CONTROL_EOF_GRACE);
                dispatch(AppControlCommand::Kill);
                return Err(error);
            }
        }
    }
}

fn force_kill_self() {
    // This code is executing inside the exact helper process. Even if another
    // thread exits the process concurrently, it cannot continue from here with
    // a recycled PID and accidentally target an unrelated process.
    let result = unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
    if result != 0 {
        log::error!(
            "failed to force-stop helper from app control channel: {}",
            io::Error::last_os_error()
        );
        std::process::abort();
    }
}

fn start_app_control_stdin(
    shutdown: tokio::sync::mpsc::UnboundedSender<AppControlCommand>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("cowchat-app-control".to_owned())
        .spawn(move || {
            let stdin = io::stdin();
            let dispatch = |command| match command {
                AppControlCommand::Interrupt | AppControlCommand::Terminate => {
                    let _ = shutdown.send(command);
                }
                AppControlCommand::Kill => force_kill_self(),
            };
            if let Err(error) = monitor_app_control(stdin.lock(), dispatch, thread::sleep) {
                log::error!("app control channel failed: {error}");
            }
        })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            socket,
            tcp,
            no_tcp,
            http,
            enable_http_signup,
            http_admin_secret,
            http_origins,
            trusted_proxy_ips,
            no_auth,
            require_local_auth,
            db,
            key_file,
            app_control_stdin,
        } => {
            let config = ServerConfig {
                socket_path: socket,
                tcp_addr: if no_tcp { None } else { Some(tcp) },
                http_addr: http.clone(),
                db_path: db,
                auth_key_path: key_file,
                no_auth,
                allow_keyless_local: !require_local_auth,
                allow_private_webhooks: false,
                http_signup_enabled: enable_http_signup,
                http_admin_secret,
                http_allowed_origins: http_origins,
                trusted_proxy_ips,
            };

            let server = CowchatServer::new(config)?;
            if no_auth {
                log::info!("Running in NO-AUTH mode (open access)");
            } else {
                if require_local_auth {
                    log::info!("Local API-key authentication is required");
                } else {
                    log::info!("Local UDS and loopback TCP connections are keyless");
                }
                if http.is_some() {
                    log::info!(
                        "API key for remote HTTP/WebSocket clients: {}",
                        server.api_key()
                    );
                }
            }
            if app_control_stdin {
                let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::unbounded_channel();
                let _app_control_thread = start_app_control_stdin(shutdown_tx)?;
                tokio::select! {
                    result = server.run() => result?,
                    command = shutdown_rx.recv() => {
                        match command {
                            Some(command) => log::info!(
                                "Shutting down from Cowchat app control command: {:?}",
                                command
                            ),
                            None => log::warn!(
                                "Cowchat app control channel ended; shutting down helper"
                            ),
                        }
                    }
                }
            } else {
                server.run().await?;
            }
        }
        Commands::Auth { action } => match action {
            AuthAction::ShowKey { key_file } => {
                let key = auth::load_or_create_key(&key_file)?;
                println!("{}", key);
            }
            AuthAction::RotateKey { key_file, db } => {
                let (db, _maintenance_lock) = lock_database_for_maintenance(&db)?;
                // Acquire the same lock as the server before touching the key
                // file. If the server is still running, rotation must leave
                // both the credential and ownership database unchanged.
                let old_key = auth::load_or_create_key(&key_file)?;
                let store = Store::open(&db)?;
                store.primary_credential_principal(&old_key)?;
                let key = auth::rotate_key(&key_file)?;
                println!("New API key: {}", key);
                println!(
                    "Stable agent, room, and subscription ownership was preserved in {}.",
                    db.display()
                );
                println!("Restart the server, then reconnect agents with the new key.");
            }
            AuthAction::MigratePrimaryOwnership {
                previous_key_file,
                db,
            } => {
                let (db, _maintenance_lock) = lock_database_for_maintenance(&db)?;
                let store = Store::open(&db)?;
                let previous_key = auth::load_existing_key(&previous_key_file)?;
                let principal = store.primary_credential_principal(&previous_key)?;
                println!(
                    "Migrated ownership for the previous primary key in {} to {}.",
                    db.display(),
                    principal
                );
                println!("Restart the server before reconnecting agents.");
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::{cell::RefCell, io::Cursor};

    #[derive(Debug, Eq, PartialEq)]
    enum ControlEvent {
        Command(AppControlCommand),
        Wait(Duration),
    }

    #[test]
    fn hidden_app_control_flag_is_parsed_for_serve() {
        let cli = Cli::try_parse_from(["cowchat-server", "serve", "--app-control-stdin"])
            .expect("hidden app control flag should parse");

        match cli.command {
            Commands::Serve {
                app_control_stdin, ..
            } => assert!(app_control_stdin),
            Commands::Auth { .. } => panic!("expected serve command"),
        }
    }

    #[test]
    fn app_control_flag_stays_out_of_serve_help() {
        let mut command = Cli::command();
        let serve = command
            .find_subcommand_mut("serve")
            .expect("serve subcommand should exist");
        let help = serve.render_long_help().to_string();

        assert!(!help.contains("app-control-stdin"));
    }

    #[test]
    fn app_control_dispatches_only_known_newline_commands() {
        let events = RefCell::new(Vec::new());
        monitor_app_control(
            Cursor::new(b"INT\nunknown\nTERM\r\nKILL\n"),
            |command| events.borrow_mut().push(ControlEvent::Command(command)),
            |duration| events.borrow_mut().push(ControlEvent::Wait(duration)),
        )
        .expect("in-memory control stream should succeed");

        assert_eq!(
            events.into_inner(),
            vec![
                ControlEvent::Command(AppControlCommand::Interrupt),
                ControlEvent::Command(AppControlCommand::Terminate),
                ControlEvent::Command(AppControlCommand::Kill),
            ]
        );
    }

    #[test]
    fn app_control_eof_terminates_then_kills_after_bounded_wait() {
        let events = RefCell::new(Vec::new());
        monitor_app_control(
            Cursor::new(Vec::<u8>::new()),
            |command| events.borrow_mut().push(ControlEvent::Command(command)),
            |duration| events.borrow_mut().push(ControlEvent::Wait(duration)),
        )
        .expect("EOF control stream should succeed");

        assert_eq!(
            events.into_inner(),
            vec![
                ControlEvent::Command(AppControlCommand::Terminate),
                ControlEvent::Wait(APP_CONTROL_EOF_GRACE),
                ControlEvent::Command(AppControlCommand::Kill),
            ]
        );
    }
}
