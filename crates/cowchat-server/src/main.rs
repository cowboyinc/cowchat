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

        /// Secret required in X-ClawChat-Admin for HTTP key creation
        /// (header name predates the rename — frozen).
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

/// True when at least one path in use is one of the exact default files.
pub(crate) fn uses_default_data_paths(
    paths_in_use: &[&std::path::Path],
    default_dir: &std::path::Path,
) -> bool {
    let defaults = ["cowchat.sock", "cowchat.db", "auth.key"].map(|f| default_dir.join(f));
    paths_in_use
        .iter()
        .any(|p| defaults.iter().any(|d| *p == d))
}

fn maybe_migrate(paths_in_use: &[&std::path::Path]) {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let new = base.home_dir().join(".cowchat");
    if !uses_default_data_paths(paths_in_use, &new) {
        return;
    }
    let old = base.home_dir().join(".clawchat");
    match cowchat_server::migrate::migrate_legacy_data_dir(&old, &new) {
        Ok(cowchat_server::migrate::MigrationOutcome::Migrated) => {
            log::info!("migrated legacy data dir ~/.clawchat -> ~/.cowchat");
        }
        Ok(cowchat_server::migrate::MigrationOutcome::NothingToDo) => {}
        Err(e) => {
            eprintln!("fatal: legacy data-dir migration failed: {e}");
            std::process::exit(1);
        }
    }
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
            db,
            key_file,
        } => {
            maybe_migrate(&[socket.as_path(), db.as_path(), key_file.as_path()]);
            let config = ServerConfig {
                socket_path: socket,
                tcp_addr: if no_tcp { None } else { Some(tcp) },
                http_addr: http,
                db_path: db,
                auth_key_path: key_file,
                no_auth,
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
                log::info!("API key: {}", server.api_key());
            }
            server.run().await?;
        }
        Commands::Auth { action } => match action {
            AuthAction::ShowKey { key_file } => {
                maybe_migrate(&[key_file.as_path()]);
                let key = auth::load_or_create_key(&key_file)?;
                println!("{}", key);
            }
            AuthAction::RotateKey { key_file } => {
                maybe_migrate(&[key_file.as_path()]);
                let key = auth::rotate_key(&key_file)?;
                println!("New API key: {}", key);
                println!("All connected agents will need to reconnect with the new key.");
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod default_path_tests {
    use super::uses_default_data_paths;
    use std::path::Path;

    #[test]
    fn predicate_matches_exact_defaults_only() {
        let dir = Path::new("/home/u/.cowchat");
        let sock = dir.join("cowchat.sock");
        let db = dir.join("cowchat.db");
        let key = dir.join("auth.key");
        let custom = dir.join("custom.db");
        let tmp = Path::new("/tmp/x/t.db");

        assert!(uses_default_data_paths(&[&sock, &db, &key], dir));
        assert!(uses_default_data_paths(&[tmp, &db], dir));
        assert!(!uses_default_data_paths(&[tmp, &custom], dir));
        assert!(
            !uses_default_data_paths(&[&custom], dir),
            "prefix is not enough"
        );
    }
}
