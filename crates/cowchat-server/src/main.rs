use clap::{Parser, Subcommand};
use cowchat_server::{auth, CowchatServer, ServerConfig};
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
    /// Provision new private hosted control/archive volumes (no listener).
    #[cfg(feature = "hosted-bootstrap")]
    HostedInit {
        #[arg(long)]
        config: PathBuf,
        /// Explicit reserve for EACH of the two volumes.
        #[arg(long)]
        reserve_wei: u128,
        #[arg(long, default_value_t = 2)]
        erasure_k: u8,
        #[arg(long, default_value_t = 1)]
        erasure_m: u8,
    },
    /// Claim one expected writer epoch, recover, then serve hosted rooms.
    #[cfg(feature = "hosted-bootstrap")]
    HostedServe {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        expected_epoch: u64,
    },
    /// Create and activate one prepared initial room, then serve it.
    #[cfg(feature = "room-key-demo")]
    HostedRoomDemo {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        expected_epoch: u64,
        /// Owner-signed initial room input; contains no plaintext room key.
        #[arg(long)]
        input: PathBuf,
    },
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

        /// Allow POST /api/keys. Open self-serve (per-IP rate-limited) unless
        /// --http-admin-secret gates it.
        #[arg(long)]
        enable_http_signup: bool,

        /// Secret required in X-Cowchat-Admin for HTTP key creation. Omit to
        /// leave signup open when --enable-http-signup is set.
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

        /// Allow webhook/wake destinations on private and loopback addresses.
        /// Required for local `cowchat actor-host` receivers; leave off for
        /// Internet-facing servers (SSRF protection).
        #[arg(long)]
        allow_private_webhooks: bool,

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    #[cfg(feature = "hosted-bootstrap")]
    let prepared = match &cli.command {
        Commands::HostedInit { config, .. } | Commands::HostedServe { config, .. } => {
            let config = cowchat_server::hosted_bootstrap::Config::load(config)?;
            let guard = config.prepare_state()?;
            // This is the single-threaded process entrypoint, before logger or
            // Tokio initialization. These SDK directories are worker-local.
            std::env::set_var(
                "CBFS_PENDING_DIFF_DIR",
                config.worker_dir.join("cbfs-pending"),
            );
            std::env::set_var(
                "CBFS_PATH_TAG_KEY_DIR",
                config.worker_dir.join("cbfs-path-tags"),
            );
            Some((config, guard))
        }
        #[cfg(feature = "room-key-demo")]
        Commands::HostedRoomDemo { config, .. } => {
            let config = cowchat_server::hosted_bootstrap::Config::load(config)?;
            let guard = config.prepare_state()?;
            std::env::set_var(
                "CBFS_PENDING_DIFF_DIR",
                config.worker_dir.join("cbfs-pending"),
            );
            std::env::set_var(
                "CBFS_PATH_TAG_KEY_DIR",
                config.worker_dir.join("cbfs-path-tags"),
            );
            Some((config, guard))
        }
        _ => None,
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            match cli.command {
                #[cfg(feature = "hosted-bootstrap")]
                Commands::HostedInit {
                    reserve_wei,
                    erasure_k,
                    erasure_m,
                    ..
                } => {
                    let (config, _guard) = prepared.as_ref().expect("hosted preflight");
                    cowchat_server::hosted_bootstrap::initialize(
                        config,
                        reserve_wei,
                        erasure_k,
                        erasure_m,
                    )
                    .await?;
                    log::info!("Hosted volumes initialized at writer epoch 0; no listener started");
                }
                #[cfg(feature = "hosted-bootstrap")]
                Commands::HostedServe { expected_epoch, .. } => {
                    let (config, _guard) = prepared.as_ref().expect("hosted preflight");
                    let (runtime, deadline) =
                        cowchat_server::hosted_bootstrap::recover(config, expected_epoch).await?;
                    let server = CowchatServer::new_hosted(config.server_config(), runtime)?;
                    log::info!("Hosted recovery complete; starting bounded authenticated session");
                    tokio::select! {
                        result = server.run() => result?,
                        _ = tokio::time::sleep_until(deadline.into()) => {
                            log::info!("Hosted credential deadline reached; stopping worker");
                        }
                    }
                }
                #[cfg(feature = "room-key-demo")]
                Commands::HostedRoomDemo {
                    expected_epoch,
                    input,
                    ..
                } => {
                    let (config, _guard) = prepared.as_ref().expect("hosted preflight");
                    let (mut runtime, deadline) =
                        cowchat_server::hosted_bootstrap::recover(config, expected_epoch).await?;
                    let input = cowchat_server::hosted_bootstrap::InitialRoomDemo::load(&input)?;
                    let probe = cowchat_server::hosted_bootstrap::activate_initial_room(
                        config,
                        &mut runtime,
                        input,
                    )
                    .await?;
                    let server = CowchatServer::new_hosted(config.server_config(), runtime)?;
                    let server_task = server.run();
                    tokio::pin!(server_task);
                    let (message_id, seq) = tokio::select! {
                        result = &mut server_task => {
                            result?;
                            return Err("hosted server stopped before the room-key probe".into());
                        }
                        result = cowchat_server::hosted_bootstrap::probe_initial_room(config, probe) => result?,
                    };
                    log::info!(
                        "Initial room activated and CBSS-backed message replayed: id={message_id} seq={seq}"
                    );
                    tokio::select! {
                        result = &mut server_task => result?,
                        _ = tokio::time::sleep_until(deadline.into()) => {
                            log::info!("Hosted credential deadline reached; stopping worker");
                        }
                    }
                }
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
                    allow_private_webhooks,
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
                        allow_private_webhooks,
                        http_signup_enabled: enable_http_signup,
                        http_admin_secret,
                        http_allowed_origins: http_origins,
                        trusted_proxy_ips,
                        blob_idle_expiry_seconds:
                            cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
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
                    AuthAction::RotateKey { key_file } => {
                        let key = auth::rotate_key(&key_file)?;
                        println!("New API key: {}", key);
                        println!("All connected agents will need to reconnect with the new key.");
                    }
                },
            }

            Ok(())
        })
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

    #[cfg(feature = "hosted-bootstrap")]
    #[test]
    fn hosted_commands_require_explicit_epoch_or_provisioning_reserve() {
        assert!(Cli::try_parse_from([
            "cowchat-server",
            "hosted-serve",
            "--config",
            "/tmp/config.json"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "cowchat-server",
            "hosted-serve",
            "--config",
            "/tmp/config.json",
            "--expected-epoch",
            "0"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "cowchat-server",
            "hosted-init",
            "--config",
            "/tmp/config.json"
        ])
        .is_err());
    }

    #[test]
    fn hidden_app_control_flag_is_parsed_for_serve() {
        let cli = Cli::try_parse_from(["cowchat-server", "serve", "--app-control-stdin"])
            .expect("hidden app control flag should parse");

        match cli.command {
            Commands::Serve {
                app_control_stdin, ..
            } => assert!(app_control_stdin),
            _ => panic!("expected serve command"),
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
