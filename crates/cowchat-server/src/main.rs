use clap::{Parser, Subcommand};
use cowchat_server::{auth, CowchatServer, ServerConfig};
use std::path::PathBuf;

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
            server.run().await?;
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
}
