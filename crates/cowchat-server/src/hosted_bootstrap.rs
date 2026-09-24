//! Explicit production hosted bootstrap. No mock authority or local fallback.
use crate::{
    room_log::{
        cbfs_archive::{CbfsArchive, RecoveryLimits},
        cbqs::CbqsOwnerLog,
        intent::IntentJournal,
        ownership::WriterRegistry,
        runtime::{OwnerRuntime, RuntimeLimits, WorkerIncarnation},
    },
    ServerConfig,
};
use anyhow::{ensure, Context, Result};
use cbfs_cli::{
    auth::CowboyClient,
    authenticated_state::TrustedCheckpoint,
    config::CliMode,
    connect::{create_volume, open_volume, VolumeContext},
};
use cbfs_registry_proto::AccessMode;
use cbqs_client::{
    chain_view_v2, AuthenticatedStreamViewV2, CheckpointTrustV2, PinnedBrokerEndpoint,
    SessionConfig, SessionV2,
};
#[cfg(feature = "room-keys")]
use cbssd::room_deployment::CompiledRoomDeployment;
use cowboy_protocol_codec::{
    cbqs_v2 as wire,
    room_authority::{derive_cowchat_service_id_v1, is_cowchat_service_endpoint_v1},
    Address,
};
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::Read,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const IO_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_COWCHAT_SERVICES: usize = 256;
const MAX_COWCHAT_SERVICES_RESPONSE_BYTES: usize = 256 * 1024;
const SAFETY_MS: u64 = 30_000;
const fn default_grant_ttl_seconds() -> u64 {
    86_400
}
const fn default_max_rooms_per_wallet() -> usize {
    100
}
const fn default_max_pending_rooms_per_wallet() -> usize {
    4
}

#[cfg(feature = "room-keys")]
mod initial_room;
#[cfg(feature = "room-keys")]
pub use initial_room::BrowserInitialRoom;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub worker_dir: PathBuf,
    cbfs_state_dir: PathBuf,
    rpc_url: String,
    trusted_checkpoint_file: PathBuf,
    owner_address: String,
    chain_instance_id: String,
    stream_id: String,
    provider_address: String,
    admin_key_file: PathBuf,
    broker_url: String,
    broker_pin: Option<BrokerPin>,
    archive_volume: String,
    control_volume: String,
    api_key_file: PathBuf,
    http_addr: SocketAddr,
    http_origins: Vec<String>,
    public_ws_url: String,
    #[serde(default = "default_max_rooms_per_wallet")]
    max_rooms_per_wallet: usize,
    #[serde(default = "default_max_pending_rooms_per_wallet")]
    max_pending_rooms_per_wallet: usize,
    #[serde(default = "default_grant_ttl_seconds")]
    grant_ttl_seconds: u64,
}

/// Browser room-key coordinator. It owns only hosted service configuration;
/// generated room keys and member signing keys remain in the browser.
#[cfg(feature = "room-keys")]
#[derive(Clone)]
pub struct HostedRoomKeys {
    config: Config,
}

#[cfg(feature = "room-keys")]
impl HostedRoomKeys {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// The deployment service ID is also the Cowchat session server ID. This
    /// lets native jobs reject a valid session challenge from another service.
    pub fn service_id(&self) -> Result<[u8; 32]> {
        Ok(CompiledRoomDeployment::compiled()?.service_id())
    }

    pub fn public_ws_url(&self) -> &str {
        &self.config.public_ws_url
    }

    pub fn wallet_room_limits(&self) -> (usize, usize) {
        (
            self.config.max_rooms_per_wallet,
            self.config.max_pending_rooms_per_wallet,
        )
    }

    async fn control_root(&self) -> Result<[u8; 32]> {
        let auth = authority(&self.config).await?;
        let ctx = volume_with_access(
            &self.config,
            &auth,
            &self.config.control_volume,
            AccessMode::ReadOnly,
            false,
        )
        .await?;
        let root = ctx.volume.manifest_root().0;
        ctx.close().await;
        Ok(root)
    }

    pub async fn initial_context(
        &self,
        owner: Address,
        room_id: String,
        members: Vec<Address>,
    ) -> Result<serde_json::Value> {
        let root = self.control_root().await?;
        cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .initial_browser_context(owner, room_id, members, root)
    }

    pub async fn successor_context(
        &self,
        previous: &cowboy_protocol_codec::room_policy::SignedRoomKeyPolicyV1,
        removed_member: Address,
    ) -> Result<serde_json::Value> {
        let root = self.control_root().await?;
        cbssd::room_deployment::CompiledRoomDeployment::compiled()?.successor_browser_context(
            previous,
            removed_member,
            root,
        )
    }

    pub async fn attest_setup(&self, signed_setup: &[u8]) -> Result<Vec<String>> {
        Ok(cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .attest_browser_setup(signed_setup)
            .await?
            .into_iter()
            .map(hex::encode)
            .collect())
    }

    /// Reject stale/conflicting preparations before they enter the owner log.
    /// An exact publication from a lost-ack attempt is accepted for retry.
    pub(crate) async fn preflight(&self, input: &BrowserInitialRoom) -> Result<()> {
        initial_room::preflight_initial_room(&self.config, input).await
    }

    /// Reserve the room in the owner log. Holds the owner-write lock only for
    /// this local append; pass `submit = false` to resume a room whose records
    /// are already durable. Kept separate from [`Self::finalize`] so the caller
    /// releases the lock across the CBSS ceremony.
    pub(crate) async fn stage(
        &self,
        runtime: &mut OwnerRuntime,
        input: &BrowserInitialRoom,
        submit: bool,
    ) -> Result<()> {
        initial_room::stage_initial_room(runtime, input, submit).await
    }

    /// Publish and finalize the prepared epoch against CBSS. Touches no owner
    /// runtime, so the caller runs it with the owner-write lock released.
    pub(crate) async fn finalize(
        &self,
        input: BrowserInitialRoom,
    ) -> Result<initial_room::PreparedInitialRoom> {
        initial_room::finalize_initial_room(&self.config, input).await
    }

    /// Commit the finalized epoch into the owner log under the owner-write lock.
    pub(crate) async fn commit(
        &self,
        runtime: &mut OwnerRuntime,
        prepared: &initial_room::PreparedInitialRoom,
    ) -> Result<()> {
        initial_room::commit_initial_room(runtime, prepared).await
    }

    pub fn open_context(
        &self,
        policy: &cowboy_protocol_codec::room_policy::SignedRoomKeyPolicyV1,
        grant: &cowboy_protocol_codec::room_release::SignedRoomKeyGrantV1,
        custody: &[u8],
    ) -> Result<serde_json::Value> {
        cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .browser_open_context(policy, grant, custody)
    }

    pub async fn relay_open(&self, request: &[u8]) -> Result<Vec<String>> {
        Ok(cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .relay_browser_open(request)
            .await?
            .into_iter()
            .map(hex::encode)
            .collect())
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerPin {
    address: SocketAddr,
    certificate_der_file: PathBuf,
}

fn fixed<const N: usize>(text: &str) -> Result<[u8; N]> {
    let bytes =
        hex::decode(text.strip_prefix("0x").unwrap_or(text)).context("invalid hex identity")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid identity length"))
}

fn read_file(path: &Path, secret: bool, limit: u64) -> Result<Zeroizing<Vec<u8>>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "invalid or oversized configuration file"
    );
    if secret {
        ensure!(
            meta.permissions().mode() & 0o077 == 0
                && meta.nlink() == 1
                && meta.uid() == unsafe { libc::geteuid() },
            "credential files must be private, owned, and not hardlinked"
        );
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "configuration file grew beyond its bound"
    );
    Ok(bytes)
}

fn private_directory(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.permissions().mode() & 0o077 == 0
            && meta.uid() == unsafe { libc::geteuid() },
        "worker directories must be private owned directories"
    );
    Ok(())
}

/// Held from preflight until the process stops, including initialization. A
/// second process cannot share the worker's CBFS pending journals or local DB.
pub struct StateGuard(File);
impl Drop for StateGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&read_file(path, false, 64 * 1024)?)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for path in [
            &self.worker_dir,
            &self.cbfs_state_dir,
            &self.trusted_checkpoint_file,
            &self.admin_key_file,
            &self.api_key_file,
        ] {
            ensure!(path.is_absolute(), "hosted paths must be absolute");
        }
        ensure!(
            (600..=86400).contains(&self.grant_ttl_seconds),
            "grant_ttl_seconds must be 600..=86400"
        );
        ensure!(
            self.worker_dir.join("server.sock").as_os_str().len() < 100,
            "worker directory is too long for a Unix socket"
        );
        ensure!(
            self.archive_volume != self.control_volume,
            "archive/control volume names must differ"
        );
        cbfs_cli::state::validate_volume_name(&self.archive_volume)?;
        cbfs_cli::state::validate_volume_name(&self.control_volume)?;
        fixed::<20>(&self.owner_address)?;
        fixed::<20>(&self.provider_address)?;
        fixed::<32>(&self.chain_instance_id)?;
        fixed::<32>(&self.stream_id)?;
        for (value, scheme) in [(&self.rpc_url, "http"), (&self.broker_url, "wss")] {
            let url = reqwest::Url::parse(value)?;
            ensure!(
                (url.scheme() == scheme || (scheme == "http" && url.scheme() == "https"))
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "invalid hosted endpoint"
            );
        }
        if let Some(pin) = &self.broker_pin {
            ensure!(
                pin.certificate_der_file.is_absolute(),
                "TLS certificate path must be absolute"
            );
            PinnedBrokerEndpoint::new(
                &self.broker_url,
                pin.address,
                &read_file(&pin.certificate_der_file, false, 16 * 1024)?,
            )
            .map_err(|_| anyhow::anyhow!("invalid pinned broker endpoint"))?;
        }
        ensure!(
            self.http_addr.port() != 0,
            "hosted HTTP port must be explicit"
        );
        let public_ws = reqwest::Url::parse(&self.public_ws_url)?;
        let loopback = public_ws.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        ensure!(
            (public_ws.scheme() == "wss" || (public_ws.scheme() == "ws" && loopback))
                && public_ws.host_str().is_some()
                && public_ws.username().is_empty()
                && public_ws.password().is_none()
                && public_ws.path() == "/ws"
                && public_ws.query().is_none()
                && public_ws.fragment().is_none(),
            "public_ws_url must be wss://, or ws:// on loopback, with path /ws"
        );
        ensure!(
            (1..=10_000).contains(&self.max_rooms_per_wallet)
                && (1..=100).contains(&self.max_pending_rooms_per_wallet),
            "invalid per-wallet room limits"
        );
        let key = read_file(&self.api_key_file, true, 4096)?;
        ensure!(
            !std::str::from_utf8(&key)?.trim().is_empty(),
            "hosted API key must already exist"
        );
        Ok(())
    }

    /// Call before constructing Tokio or any other threads. The CLI sets the
    /// SDK environment to these private directories while this guard is held.
    pub fn prepare_state(&self) -> Result<StateGuard> {
        private_directory(&self.worker_dir)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.worker_dir.join("worker.lock"))?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file()
                && meta.nlink() == 1
                && meta.permissions().mode() & 0o077 == 0
                && meta.uid() == unsafe { libc::geteuid() },
            "unsafe worker lock file"
        );
        fs2::FileExt::try_lock_exclusive(&file)
            .context("another process owns this worker directory")?;
        let guard = StateGuard(file);
        private_directory(&self.worker_dir.join("cbfs-pending"))?;
        private_directory(&self.worker_dir.join("cbfs-path-tags"))?;
        ensure!(
            std::fs::canonicalize(&self.worker_dir)?
                != std::fs::canonicalize(&self.cbfs_state_dir)?,
            "worker and CBFS credential directories must differ"
        );
        Ok(guard)
    }

    fn check_environment(&self) -> Result<()> {
        for (name, subdir) in [
            ("CBFS_PENDING_DIFF_DIR", "cbfs-pending"),
            ("CBFS_PATH_TAG_KEY_DIR", "cbfs-path-tags"),
        ] {
            ensure!(
                std::env::var_os(name).as_deref() == Some(self.worker_dir.join(subdir).as_os_str()),
                "hosted SDK state environment was not isolated before startup"
            );
        }
        Ok(())
    }

    pub fn server_config(&self) -> ServerConfig {
        ServerConfig {
            socket_path: self.worker_dir.join("server.sock"),
            tcp_addr: None,
            http_addr: Some(self.http_addr.to_string()),
            db_path: self.worker_dir.join("auth.sqlite"),
            auth_key_path: self.api_key_file.clone(),
            no_auth: false,
            allow_keyless_local: false,
            allow_private_webhooks: false,
            http_signup_enabled: false,
            http_admin_secret: None,
            http_allowed_origins: self.http_origins.clone(),
            trusted_proxy_ips: vec![],
            blob_idle_expiry_seconds: crate::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
        }
    }
}

struct Authority {
    cbfs: CowboyClient,
    checkpoint: TrustedCheckpoint,
    view: AuthenticatedStreamViewV2,
    admin: SigningKey,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CowchatServicesResponse {
    services: Vec<CowchatServiceResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CowchatServiceResponse {
    service_id: String,
    operator: String,
    endpoint: String,
    #[serde(rename = "registered_at_block")]
    _registered_at_block: u64,
}

fn canonical_rpc_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    ensure!(
        value.len() == 2 + N * 2
            && value.starts_with("0x")
            && value[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid {label}"
    );
    hex::decode(&value[2..])?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid {label}"))
}

async fn fetch_cowchat_services(rpc_url: &str) -> Result<CowchatServicesResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client
        .get(format!(
            "{}/cowchat/services",
            rpc_url.trim_end_matches('/')
        ))
        .send()
        .await?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Cowchat service registry request failed"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            body.len()
                .checked_add(chunk.len())
                .is_some_and(|size| size <= MAX_COWCHAT_SERVICES_RESPONSE_BYTES),
            "Cowchat service registry response exceeds bound"
        );
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).context("invalid Cowchat service registry response")
}

fn verify_cowchat_services(
    response: CowchatServicesResponse,
    chain_id: u64,
    expected_operator: &Address,
    expected_endpoint: &str,
) -> Result<()> {
    ensure!(
        response.services.len() <= MAX_COWCHAT_SERVICES,
        "Cowchat service registry exceeds entry bound"
    );
    let expected_service_id = derive_cowchat_service_id_v1(chain_id, expected_operator);
    let mut seen = BTreeSet::new();
    let mut matched = false;
    for service in response.services {
        let CowchatServiceResponse {
            service_id,
            operator,
            endpoint,
            _registered_at_block: _,
        } = service;
        let service_id = canonical_rpc_hex::<32>(&service_id, "Cowchat service ID")?;
        let operator = canonical_rpc_hex::<20>(&operator, "Cowchat service operator")?;
        let operator = Address::from_bytes(operator);
        ensure!(
            is_cowchat_service_endpoint_v1(endpoint.as_bytes()),
            "invalid Cowchat service endpoint"
        );
        ensure!(
            service_id == derive_cowchat_service_id_v1(chain_id, &operator),
            "Cowchat service ID does not match its operator"
        );
        ensure!(
            seen.insert(service_id),
            "duplicate Cowchat service registry entry"
        );
        if service_id == expected_service_id {
            ensure!(
                &operator == expected_operator,
                "Cowchat service operator mismatch"
            );
            ensure!(
                endpoint == expected_endpoint,
                "Cowchat service endpoint mismatch"
            );
            matched = true;
        }
    }
    ensure!(matched, "Cowchat service is not registered");
    Ok(())
}

async fn verify_cowchat_service_registration(
    config: &Config,
    chain_id: u64,
    operator: Address,
) -> Result<()> {
    #[cfg(feature = "room-keys")]
    ensure!(
        CompiledRoomDeployment::compiled()?.service_id()
            == derive_cowchat_service_id_v1(chain_id, &operator),
        "compiled room deployment service ID does not match the registered Cowchat service"
    );
    let response = fetch_cowchat_services(&config.rpc_url).await?;
    verify_cowchat_services(response, chain_id, &operator, &config.public_ws_url)
}

fn now_ms() -> Result<u64> {
    u64::try_from(chrono::Utc::now().timestamp_millis()).context("clock predates Unix epoch")
}

async fn authority(config: &Config) -> Result<Authority> {
    config.check_environment()?;
    let cbfs = CowboyClient::load(&config.cbfs_state_dir, Some(&config.rpc_url))?;
    let owner = fixed::<20>(&config.owner_address)?;
    ensure!(
        cbfs.delegation.wallet_address.as_bytes() == &owner,
        "CBFS credentials do not belong to configured owner"
    );
    let secret = read_file(&config.admin_key_file, true, 256)?;
    let seed = Zeroizing::new(fixed::<32>(std::str::from_utf8(&secret)?.trim())?);
    let admin = SigningKey::from_bytes(&seed);
    let anchor = read_file(&config.trusted_checkpoint_file, false, 1024 * 1024)?;
    let checkpoint = TrustedCheckpoint::from_bytes(anchor.to_vec())?;
    let stream = fixed(&config.stream_id)?;
    let provider = Address::from_bytes(fixed(&config.provider_address)?);
    let instance = fixed(&config.chain_instance_id)?;
    let actor = format!(
        "0x{}",
        hex::encode(chain_view_v2::STREAM_REGISTRY_SYSTEM_ACTOR)
    );
    // The node requires claims strictly sorted by (actor, logical key); both
    // share the registry actor, and `cbqs:provider:` sorts before `cbqs:stream:`.
    let mut keys = [
        chain_view_v2::stream_key(&stream),
        chain_view_v2::provider_key(&provider),
    ];
    keys.sort();
    let claims = keys
        .iter()
        .map(|key| serde_json::json!({"actor":actor,"logical_key_hex":format!("0x{}",hex::encode(key))}))
        .collect::<Vec<_>>();
    let body = serde_json::json!({"checkpoint_height":checkpoint.height(),"bundle_version":2,
        "claims":claims});
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client
        .post(format!(
            "{}/proof/finalized-state",
            config.rpc_url.trim_end_matches('/')
        ))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let mut proof = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            proof.len().saturating_add(chunk.len()) <= 32 * 1024 * 1024,
            "finalized proof exceeds bound"
        );
        proof.extend_from_slice(&chunk);
    }
    let view = chain_view_v2::authenticate(&anchor, &proof, &stream, &provider, &instance)?;
    ensure!(
        view.chain_id() == cbfs.delegation.chain_id && view.stream().owner.as_bytes() == &owner,
        "proved chain/stream owner mismatch"
    );
    ensure!(
        view.stream().status == wire::StreamStatusV2::Active,
        "proved stream is not active"
    );
    ensure!(
        view.stream().admin_key.algorithm == wire::SigningKeyAlgorithmV2::Ed25519
            && view.stream().admin_key.key_bytes == admin.verifying_key().to_bytes(),
        "admin key does not match proved stream authority"
    );
    ensure!(
        view.provider()
            .endpoints
            .iter()
            .any(|endpoint| endpoint == config.broker_url.as_bytes()),
        "broker URL not present in proved provider record"
    );
    let now = now_ms()?;
    ensure!(
        view.target_timestamp_ms() <= now.saturating_add(30_000)
            && now.saturating_sub(view.target_timestamp_ms()) <= 120_000,
        "finalized proof is not recent"
    );
    ensure!(
        cbfs.delegation.expires_at_ms > now.saturating_add(120_000),
        "CBFS delegation is near expiry"
    );
    Ok(Authority {
        cbfs,
        checkpoint,
        view,
        admin,
    })
}

async fn volume_with_access(
    config: &Config,
    auth: &Authority,
    name: &str,
    access: AccessMode,
    mount: bool,
) -> Result<VolumeContext> {
    // A hosted writer is a long-lived owner session, using the existing mount
    // token lifetime; renewal retains the access mode recorded in the context.
    let ctx = open_volume(
        &config.cbfs_state_dir,
        name,
        CliMode::Cowboy,
        Some(&config.rpc_url),
        access,
        mount,
        5,
        Some(&auth.checkpoint),
    )
    .await?;
    let meta = ctx
        .cached_open_response
        .as_ref()
        .context("missing Cowboy volume metadata")?;
    ensure!(
        ctx.http_config.is_some()
            && meta.owner_address == auth.cbfs.delegation.wallet_address
            && meta.visibility == cbfs_registry_proto::Visibility::Private,
        "hosted volume must be private and owned by the stream owner"
    );
    ensure!(
        ctx.initial_token_expires_at_ms
            .is_some_and(|expiry| expiry > now_ms().unwrap_or(u64::MAX).saturating_add(120_000)),
        "CBFS attachment is near expiry"
    );
    Ok(ctx)
}

async fn volume(config: &Config, auth: &Authority, name: &str) -> Result<VolumeContext> {
    volume_with_access(config, auth, name, AccessMode::ReadWrite, true).await
}

/// Provisioning never adopts an existing empty volume. A partial failure keeps
/// its created volume for diagnosis; choose new names rather than reset it.
pub async fn initialize(config: &Config, reserve_wei: u128, k: u8, m: u8) -> Result<()> {
    ensure!(
        reserve_wei > 0 && k > 0 && m > 0 && u16::from(k) + u16::from(m) <= 255,
        "invalid provisioning reserve/layout"
    );
    let auth = authority(config).await?;
    for name in [&config.control_volume, &config.archive_volume] {
        ensure!(
            cbfs_cli::state::try_load(&config.cbfs_state_dir, name)?.is_none(),
            "provisioning requires unused volume names"
        );
    }
    let instance = auth.view.chain_instance_id();
    let stream = auth.view.stream().stream_id;
    for (name, control) in [
        (&config.control_volume, true),
        (&config.archive_volume, false),
    ] {
        let created = create_volume(
            &config.cbfs_state_dir,
            name,
            &[],
            k,
            m,
            cbfs_types::Visibility::Private,
            CliMode::Cowboy,
            Some(&config.rpc_url),
            reserve_wei,
            true,
            Some(&auth.checkpoint),
        )
        .await?;
        let ctx = volume(config, &auth, name).await?;
        ensure!(
            ctx.volume.volume_id().0 == created.volume_id,
            "created volume identity changed"
        );
        if control {
            WriterRegistry::initialize_new_volume(
                ctx.volume,
                ctx.auth_store,
                ctx.registry,
                instance,
                stream,
                IO_TIMEOUT,
            )
            .await?;
        } else {
            CbfsArchive::initialize_new_volume(
                ctx.volume,
                ctx.auth_store,
                ctx.registry,
                instance,
                stream,
                MAX_BYTES,
                IO_TIMEOUT,
            )
            .await?;
        }
    }
    Ok(())
}

fn mint_grant(
    view: &AuthenticatedStreamViewV2,
    admin: &SigningKey,
    holder: &SigningKey,
    epoch: u64,
    now: u64,
    expiry: u64,
) -> wire::StreamGrantV2 {
    let grant = wire::StreamGrantV2 {
        version: wire::CBQS_VERSION_V2,
        chain_instance_id: view.chain_instance_id(),
        stream_id: view.stream().stream_id,
        authorization_generation: view.stream().authorization_generation,
        policy_epoch: epoch,
        grant_nonce: rand::random(),
        holder_signing_key: wire::SigningPublicKeyV2 {
            algorithm: wire::SigningKeyAlgorithmV2::Ed25519,
            key_bytes: holder.verifying_key().to_bytes(),
        },
        verbs: wire::CBQS_V2_VERB_APPEND
            | wire::CBQS_V2_VERB_REPLAY
            | wire::CBQS_V2_VERB_CONSUME
            | wire::CBQS_V2_VERB_LANE_ADMIN,
        lane_scope: wire::LaneScopeV2::Any,
        group_scope: wire::GroupScopeV2::Any,
        not_before_ms: now,
        expires_at_ms: expiry,
        max_message_bytes: 262_144,
        max_append_bytes_per_sec: 1_048_576,
        signature: wire::CbqsSignatureV2([0; 64]),
    };
    remint_grant(grant, admin, now, expiry)
}

fn remint_grant(
    mut grant: wire::StreamGrantV2,
    admin: &SigningKey,
    now: u64,
    expiry: u64,
) -> wire::StreamGrantV2 {
    grant.grant_nonce = rand::random();
    grant.not_before_ms = now;
    grant.expires_at_ms = expiry;
    grant.signature = wire::CbqsSignatureV2(
        admin
            .sign(&cowboy_protocol_codec::keccak256(
                &wire::stream_grant_signing_bytes_v2(&grant),
            ))
            .to_bytes(),
    );
    grant
}

async fn connect_broker(config: &Config) -> Result<cbqs_client::Socket> {
    if let Some(pin) = &config.broker_pin {
        let endpoint = PinnedBrokerEndpoint::new(
            &config.broker_url,
            pin.address,
            &read_file(&pin.certificate_der_file, false, 16 * 1024)?,
        )
        .map_err(|_| anyhow::anyhow!("invalid pinned broker endpoint"))?;
        endpoint
            .connect_socket(&config.broker_url)
            .await
            .map_err(|error| anyhow::anyhow!("pinned broker connection failed: {error:?}"))
    } else {
        let url = reqwest::Url::parse(&config.broker_url)?;
        let addresses: Vec<_> = tokio::net::lookup_host((
            url.host_str().context("missing broker host")?,
            url.port_or_known_default().unwrap_or(443),
        ))
        .await?
        .map(|a| a.ip())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
        cbqs_client::transport_v2::connect_checked(&config.broker_url, &addresses)
            .await
            .map_err(|error| anyhow::anyhow!("checked broker connection failed: {error:?}"))
    }
}

async fn attach(
    config: &Config,
    auth: &Authority,
    worker: &WorkerIncarnation,
    epoch: u64,
    expiry: u64,
) -> Result<CbqsOwnerLog> {
    let grant = mint_grant(
        &auth.view,
        &auth.admin,
        worker.holder(),
        epoch,
        now_ms()?,
        expiry,
    );
    let fence = connect_broker(config).await?;
    let session = connect_broker(config).await?;
    Ok(CbqsOwnerLog::attach(
        fence,
        session,
        SessionConfig {
            broker_url: config.broker_url.clone(),
            grant,
            holder: worker.holder().clone(),
            handshake_timeout: Duration::from_secs(30),
            checkpoints: CheckpointTrustV2::ChainProvider(Box::new(auth.view.provider().clone())),
        },
        |bytes| {
            auth.admin
                .sign(&cowboy_protocol_codec::keccak256(bytes))
                .to_bytes()
        },
        now_ms()?,
    )
    .await?)
}

/// A renewal failure either may clear before the horizon, or proves this
/// writer's authority is gone. Only a restart (claim, fence, replay) recovers
/// from the second, so the loop retires the view and returns it.
pub(crate) enum Failure {
    Retry(anyhow::Error),
    Terminal(anyhow::Error),
}

/// Broker refusals that no retry of the same authority can clear.
fn terminal_attach(error: &cbqs_client::TransportError) -> bool {
    matches!(error, cbqs_client::TransportError::Broker(error) if matches!(
        error.code,
        wire::CBQS_V2_ERR_STREAM_NOT_ACTIVE
            | wire::CBQS_V2_ERR_AUTHORIZATION_GENERATION_STALE
            | wire::CBQS_V2_ERR_INVALID_GRANT
            | wire::CBQS_V2_ERR_POLICY_EPOCH_STALE
    ))
}

/// Mints a same-epoch grant and attaches its session on a new connection.
/// A seam only so the renewal loop can run against the fixture broker.
pub(crate) trait Issue {
    fn attach(
        &mut self,
        now: u64,
        expiry: u64,
    ) -> impl std::future::Future<Output = Result<(SessionV2, wire::StreamGrantV2), Failure>> + Send;
}

pub(crate) struct Issuer {
    config: Config,
    auth: Authority,
    holder: SigningKey,
    epoch: u64,
}

impl Issue for Issuer {
    async fn attach(
        &mut self,
        now: u64,
        expiry: u64,
    ) -> Result<(SessionV2, wire::StreamGrantV2), Failure> {
        let grant = mint_grant(
            &self.auth.view,
            &self.auth.admin,
            &self.holder,
            self.epoch,
            now,
            expiry,
        );
        let socket = connect_broker(&self.config).await.map_err(Failure::Retry)?;
        let session = SessionV2::attach(
            socket,
            SessionConfig {
                broker_url: self.config.broker_url.clone(),
                grant: grant.clone(),
                holder: self.holder.clone(),
                handshake_timeout: Duration::from_secs(30),
                checkpoints: CheckpointTrustV2::ChainProvider(Box::new(
                    self.auth.view.provider().clone(),
                )),
            },
            now_ms().map_err(Failure::Retry)?,
        )
        .await
        .map_err(|error| {
            let terminal = terminal_attach(&error);
            let error = anyhow::anyhow!("replacement CBQS session rejected: {error:?}");
            if terminal {
                Failure::Terminal(error)
            } else {
                Failure::Retry(error)
            }
        })?;
        Ok((session, grant))
    }
}

/// How often an idle writer proves its session still holds authority. A
/// fence or revocation otherwise surfaces only on the next write, and until
/// then this process would keep serving reads it can no longer vouch for.
const PROBE_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(15)
};

/// Credential state belongs to this writer incarnation only. Nothing is
/// persisted: a crash still follows the existing intent replay/fence path.
pub(crate) struct Renewal<I = Issuer> {
    issuer: I,
    grant_ttl_ms: u64,
    epoch: u64,
    grant_expiry: u64,
    delegation_expiry: u64,
    tokens: Vec<cbfs_cli::token_refresh::OwnerTokenRefresh>,
    horizon: Instant,
    delegation_horizon: Instant,
}

impl<I: Issue> Renewal<I> {
    pub(crate) fn new(
        issuer: I,
        grant_ttl_ms: u64,
        epoch: u64,
        grant_expiry: u64,
        delegation_expiry: u64,
        tokens: Vec<cbfs_cli::token_refresh::OwnerTokenRefresh>,
    ) -> Result<Self> {
        let instant = Instant::now();
        let delegation_remaining = delegation_expiry
            .checked_sub(now_ms()?.saturating_add(SAFETY_MS))
            .context("CBFS delegation expired during recovery")?;
        let delegation_horizon = instant + Duration::from_millis(delegation_remaining);
        let mut renewal = Self {
            issuer,
            grant_ttl_ms,
            epoch,
            grant_expiry,
            delegation_expiry,
            tokens,
            horizon: Instant::now(),
            delegation_horizon,
        };
        renewal.horizon = renewal.new_horizon()?;
        Ok(renewal)
    }

    pub(crate) fn horizon(&self) -> Instant {
        self.horizon
    }

    fn expiry(&self) -> u64 {
        self.tokens.iter().fold(
            self.grant_expiry.min(self.delegation_expiry),
            |expiry, token| expiry.min(token.initial_expires_at_ms),
        )
    }

    fn new_horizon(&self) -> Result<Instant> {
        // Sample monotonic time first: conversion must never extend authority.
        let instant = Instant::now();
        let remaining = self
            .expiry()
            .checked_sub(now_ms()?.saturating_add(SAFETY_MS))
            .context("hosted credentials expired or inside safety margin")?;
        Ok((instant + Duration::from_millis(remaining)).min(self.delegation_horizon))
    }

    /// Token minting and the new broker session; no writer lock is held, so
    /// this is the part bounded by IO_TIMEOUT.
    async fn prepare(&mut self) -> Result<(SessionV2, wire::StreamGrantV2, u64), Failure> {
        for token in &mut self.tokens {
            let minted =
                (token.mint)().map_err(|error| Failure::Retry(anyhow::Error::msg(error)))?;
            if minted.expires_at_ms < token.initial_expires_at_ms {
                return Err(Failure::Retry(anyhow::anyhow!(
                    "CBFS replacement would shorten credential validity"
                )));
            }
            token.http_config.set_attachment_token(minted.token_bytes);
            token.initial_expires_at_ms = minted.expires_at_ms;
        }
        let now = now_ms().map_err(Failure::Retry)?;
        let expiry = now.saturating_add(self.grant_ttl_ms);
        let (session, grant) = self.issuer.attach(now, expiry).await?;
        Ok((session, grant, expiry))
    }

    /// Any failure here is terminal: the runtime refused the replacement or
    /// the horizon cannot advance, and neither changes on retry.
    fn install(
        &mut self,
        runtime: &mut OwnerRuntime,
        session: SessionV2,
        grant: wire::StreamGrantV2,
        expiry: u64,
    ) -> Result<()> {
        runtime.swap_session(session, grant)?;
        self.grant_expiry = expiry;
        let horizon = self.new_horizon()?;
        // The fixed delegation ceiling cannot advance with clock conversion
        // jitter. Keep the current bound if a candidate is shorter.
        if horizon > self.horizon {
            runtime.advance_horizon(horizon)?;
            self.horizon = horizon;
        }
        Ok(())
    }

    /// Returns only with an error, after retiring the view, so the process
    /// exits and its supervisor restarts it against current authority.
    pub(crate) async fn run(
        mut self,
        writer: &tokio::sync::Mutex<OwnerRuntime>,
        view: &crate::room_log::runtime::OwnerView,
    ) -> Result<()> {
        let mut backoff = Duration::from_secs(1);
        let mut next = Instant::now() + self.horizon.saturating_duration_since(Instant::now()) / 2;
        let mut probe = Instant::now() + PROBE_INTERVAL;
        let mut warned = false;
        let horizon_reached = || {
            view.retire();
            anyhow::anyhow!("Hosted credential horizon reached; renewal did not extend valid credentials; worker retired")
        };
        loop {
            let now = now_ms()?;
            if !warned && self.delegation_expiry.saturating_sub(now) <= 86_400_000 {
                log::error!("CBFS wallet-signed delegation expires at {} (within 24 hours); operator renewal and worker restart required",
                    self.delegation_expiry);
                warned = true;
            }
            let horizon = self.horizon;
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(horizon.into()) => return Err(horizon_reached()),
                _ = tokio::time::sleep_until(probe.into()) => {
                    probe = Instant::now() + PROBE_INTERVAL;
                    if let Err(error) = writer.lock().await.probe().await {
                        view.retire();
                        anyhow::bail!("Hosted writer lost broker authority: {error}; worker retired");
                    }
                    continue;
                }
                _ = tokio::time::sleep_until(next.into()) => {}
            }
            let prepared = tokio::select! {
                biased;
                _ = tokio::time::sleep_until(horizon.into()) => return Err(horizon_reached()),
                result = tokio::time::timeout(IO_TIMEOUT, self.prepare()) => result.unwrap_or_else(|_| {
                    Err(Failure::Retry(anyhow::anyhow!("hosted credential renewal timed out")))
                }),
            };
            let result = match prepared {
                Ok((session, grant, expiry)) => {
                    let mut runtime = writer.lock().await;
                    self.install(&mut runtime, session, grant, expiry)
                        .map_err(Failure::Terminal)
                }
                Err(failure) => Err(failure),
            };
            match result {
                Ok(()) => {
                    backoff = Duration::from_secs(1);
                    // Half the remaining safe lifetime is before 2/3 grant TTL,
                    // and also respects shorter CBFS credentials/delegation.
                    let delay = self.horizon.saturating_duration_since(Instant::now()) / 2;
                    let delay = delay.max(Duration::from_secs(1));
                    next = Instant::now() + delay;
                    log::info!(
                        "Hosted credentials renewed at writer epoch {}; next renewal in {}s",
                        self.epoch,
                        delay.as_secs()
                    );
                }
                Err(Failure::Retry(error)) => {
                    log::error!(
                        "Hosted credential renewal failed: {error:#}; retrying in {backoff:?}"
                    );
                    next = Instant::now() + backoff;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
                Err(Failure::Terminal(error)) => {
                    view.retire();
                    return Err(error.context("Hosted credential renewal refused; worker retired"));
                }
            }
        }
    }
}

/// The caller supplies the one expected epoch. Never auto-increment/retry a
/// losing CAS. Recover fully before the caller constructs any listener.
pub async fn recover(config: &Config, expected_epoch: u64) -> Result<OwnerRuntime> {
    ensure!(expected_epoch < u64::MAX - 1, "expected epoch is exhausted");
    let auth = authority(config).await?;
    // `authority` proved that this configured owner is both the CBFS wallet and
    // the stream owner. Refuse to serve under an unregistered endpoint before
    // opening local journals, attaching volumes, or claiming a writer epoch.
    verify_cowchat_service_registration(
        config,
        auth.view.chain_id(),
        Address::from_bytes(*auth.view.stream().owner.as_bytes()),
    )
    .await?;
    let owner = format!("0x{}", hex::encode(auth.view.stream().owner.as_bytes()));
    let instance = auth.view.chain_instance_id();
    let stream = auth.view.stream().stream_id;
    let journal = IntentJournal::open(
        &config.worker_dir.join("intents.sqlite"),
        owner.clone(),
        instance,
        stream,
        100,
        MAX_BYTES,
    )?;
    let ctx = volume(config, &auth, &config.control_volume).await?;
    let control_refresh = cbfs_cli::token_refresh::build_owner_token_refresh(
        &config.cbfs_state_dir,
        Some(&config.rpc_url),
        &ctx,
        5,
    )?;
    let mut registry = WriterRegistry::open(
        ctx.volume,
        ctx.auth_store,
        ctx.registry,
        instance,
        stream,
        IO_TIMEOUT,
    )
    .await?;
    let observed_epoch = registry.epoch()?;
    ensure!(observed_epoch == expected_epoch,
        "control epoch mismatch: expected {expected_epoch}, observed {observed_epoch}; no claim attempted");
    let worker = WorkerIncarnation::fresh();
    let allocation = registry
        .claim(expected_epoch, worker.writer_id(), worker.claim_id())
        .await?;
    log::info!(
        "Acquired hosted writer epoch {}; fencing before recovery",
        allocation.epoch()
    );
    // Sampled after the claim so claim latency never shortens the first grant.
    let grant_expiry = now_ms()?.saturating_add(config.grant_ttl_seconds * 1000);
    let log = tokio::time::timeout(
        IO_TIMEOUT,
        attach(config, &auth, &worker, allocation.epoch(), grant_expiry),
    )
    .await??;
    let epoch = allocation.epoch();
    let holder = worker.holder().clone();
    let writer = worker.bind(allocation, log)?;
    // Open archive AFTER fencing the previous writer, so its final committed
    // root participates in recovery rather than a pre-fence snapshot.
    let ctx = volume(config, &auth, &config.archive_volume).await?;
    let archive_refresh = cbfs_cli::token_refresh::build_owner_token_refresh(
        &config.cbfs_state_dir,
        Some(&config.rpc_url),
        &ctx,
        5,
    )?;
    let archive = CbfsArchive::open(
        ctx.volume,
        ctx.auth_store,
        ctx.registry,
        instance,
        stream,
        MAX_BYTES,
        IO_TIMEOUT,
    )
    .await?;
    let mut runtime = OwnerRuntime::recover(
        owner,
        writer,
        archive,
        journal,
        RuntimeLimits {
            archive: RecoveryLimits {
                max_segments: 10_000,
                max_records: 100_000,
                max_bytes: 256 * 1024 * 1024,
            },
            batch_records: 100,
            replay_bytes: MAX_BYTES,
        },
    )
    .await?;
    let delegation_expiry = auth.cbfs.delegation.expires_at_ms;
    let renewal = Renewal::new(
        Issuer {
            config: config.clone(),
            auth,
            holder,
            epoch,
        },
        config.grant_ttl_seconds * 1000,
        epoch,
        grant_expiry,
        delegation_expiry,
        vec![control_refresh, archive_refresh],
    )?;
    runtime.advance_horizon(renewal.horizon())?;
    runtime.renewal = Some(renewal);
    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Router};

    fn config() -> (tempfile::TempDir, Config) {
        let dir = tempfile::Builder::new()
            .prefix("cc")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let cbfs = dir.path().join("credentials");
        private_directory(&cbfs).unwrap();
        let key = dir.path().join("api.key");
        std::fs::write(&key, "test-only-api-key").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let config = Config {
            worker_dir: dir.path().join("worker"),
            cbfs_state_dir: cbfs,
            rpc_url: "http://127.0.0.1:1".into(),
            trusted_checkpoint_file: dir.path().join("checkpoint.bin"),
            owner_address: "01".repeat(20),
            chain_instance_id: "02".repeat(32),
            stream_id: "03".repeat(32),
            provider_address: "04".repeat(20),
            admin_key_file: dir.path().join("admin.key"),
            broker_url: "wss://broker.example/ws".into(),
            broker_pin: None,
            archive_volume: "archive".into(),
            control_volume: "control".into(),
            api_key_file: key,
            http_addr: "127.0.0.1:19440".parse().unwrap(),
            http_origins: vec![],
            public_ws_url: "ws://127.0.0.1:19440/ws".into(),
            max_rooms_per_wallet: default_max_rooms_per_wallet(),
            max_pending_rooms_per_wallet: default_max_pending_rooms_per_wallet(),
            grant_ttl_seconds: 600,
        };
        (dir, config)
    }

    #[test]
    fn grant_ttl_bounds_allow_a_day_without_capping_process_lifetime() {
        let (_directory, mut config) = config();
        assert_eq!(default_grant_ttl_seconds(), 86_400);
        for ttl in [600, 900, 3601, 86400] {
            config.grant_ttl_seconds = ttl;
            config.validate().unwrap();
        }
        for ttl in [0, 120, 599, 86401] {
            config.grant_ttl_seconds = ttl;
            assert!(config.validate().is_err());
        }
    }

    async fn registry_server(status: StatusCode, body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/cowchat/services",
            get(move || {
                let body = body.clone();
                async move { (status, body) }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    fn service_json(chain_id: u64, operator: &Address, endpoint: &str) -> serde_json::Value {
        serde_json::json!({
            "service_id": format!(
                "0x{}",
                hex::encode(derive_cowchat_service_id_v1(chain_id, operator))
            ),
            "operator": format!("0x{}", hex::encode(operator.as_bytes())),
            "endpoint": endpoint,
            "registered_at_block": 7,
        })
    }

    #[tokio::test]
    async fn cowchat_registry_accepts_only_the_exact_registered_service() {
        let chain_id = 9;
        let operator = Address::from_bytes([0x22; 20]);
        let endpoint = "wss://chat.example/ws";
        let body = serde_json::to_vec(&serde_json::json!({
            "services": [service_json(chain_id, &operator, endpoint)]
        }))
        .unwrap();
        let rpc_url = registry_server(StatusCode::OK, body).await;

        let response = fetch_cowchat_services(&rpc_url).await.unwrap();
        verify_cowchat_services(response, chain_id, &operator, endpoint).unwrap();
    }

    #[test]
    fn cowchat_registry_rejects_missing_mismatched_and_noncanonical_records() {
        let chain_id = 9;
        let operator = Address::from_bytes([0x22; 20]);
        let endpoint = "wss://chat.example/ws";
        let exact = service_json(chain_id, &operator, endpoint);
        let mut wrong_endpoint = exact.clone();
        wrong_endpoint["endpoint"] = serde_json::json!("wss://other.example/ws");
        let mut wrong_id = exact.clone();
        wrong_id["service_id"] = serde_json::json!(format!("0x{}", "00".repeat(32)));
        let mut uppercase_id = exact.clone();
        uppercase_id["service_id"] = serde_json::json!(exact["service_id"]
            .as_str()
            .unwrap()
            .to_ascii_uppercase()
            .replacen("0X", "0x", 1));

        for services in [
            Vec::new(),
            vec![wrong_endpoint],
            vec![wrong_id],
            vec![uppercase_id],
            vec![exact.clone(), exact],
        ] {
            let response: CowchatServicesResponse =
                serde_json::from_value(serde_json::json!({"services": services})).unwrap();
            assert!(verify_cowchat_services(response, chain_id, &operator, endpoint).is_err());
        }

        let services = (0..=MAX_COWCHAT_SERVICES)
            .map(|index| service_json(chain_id, &Address::from_bytes([index as u8; 20]), endpoint))
            .collect::<Vec<_>>();
        let response: CowchatServicesResponse =
            serde_json::from_value(serde_json::json!({"services": services})).unwrap();
        assert!(verify_cowchat_services(response, chain_id, &operator, endpoint).is_err());
    }

    #[tokio::test]
    async fn cowchat_registry_fetch_rejects_non_success_malformed_and_oversized_bodies() {
        let valid = serde_json::to_vec(&serde_json::json!({"services": []})).unwrap();
        for (status, body) in [
            (StatusCode::FOUND, valid),
            (
                StatusCode::CREATED,
                serde_json::to_vec(&serde_json::json!({"services": []})).unwrap(),
            ),
            (StatusCode::OK, b"not-json".to_vec()),
            (
                StatusCode::OK,
                vec![b' '; MAX_COWCHAT_SERVICES_RESPONSE_BYTES + 1],
            ),
        ] {
            let rpc_url = registry_server(status, body).await;
            assert!(fetch_cowchat_services(&rpc_url).await.is_err());
        }
    }

    #[test]
    fn preflight_refuses_missing_public_or_linked_credentials_without_creating_state() {
        let (_dir, config) = config();
        config.validate().unwrap();
        std::fs::set_permissions(&config.api_key_file, std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(config.validate().is_err());
        std::fs::set_permissions(&config.api_key_file, std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let saved = config.api_key_file.with_extension("saved");
        std::fs::rename(&config.api_key_file, &saved).unwrap();
        assert!(config.validate().is_err());
        std::os::unix::fs::symlink(&saved, &config.api_key_file).unwrap();
        assert!(config.validate().is_err());
        std::fs::remove_file(&config.api_key_file).unwrap();
        std::fs::hard_link(&saved, &config.api_key_file).unwrap();
        assert!(config.validate().is_err());
        assert!(!config.worker_dir.exists());
    }

    #[test]
    fn worker_state_is_exclusive_and_rejects_symlink_aliases() {
        let (_dir, config) = config();
        let guard = config.prepare_state().unwrap();
        assert!(config.prepare_state().is_err());
        assert!(!config.worker_dir.join("auth.sqlite").exists());
        drop(guard);
        drop(config.prepare_state().unwrap());
        let pending = config.worker_dir.join("cbfs-pending");
        std::fs::remove_dir(&pending).unwrap();
        std::os::unix::fs::symlink(&config.cbfs_state_dir, &pending).unwrap();
        assert!(config.prepare_state().is_err());
    }

    #[test]
    fn hosted_config_has_no_open_auth_or_local_transport_escape() {
        let (_dir, mut config) = config();
        let server = config.server_config();
        assert!(!server.no_auth && !server.allow_keyless_local && !server.http_signup_enabled);
        assert!(server.tcp_addr.is_none());
        assert!(!server.allow_private_webhooks);
        config.broker_url = "ws://broker.example/ws".into();
        assert!(config.validate().is_err());
        config.broker_url = "wss://broker.example/ws".into();
        config.public_ws_url = "ws://chat.example/ws".into();
        assert!(config.validate().is_err());
        config.public_ws_url = "wss://chat.example/ws".into();
        config.max_rooms_per_wallet = 0;
        assert!(config.validate().is_err());
        config.max_rooms_per_wallet = default_max_rooms_per_wallet();
        config.archive_volume = config.control_volume.clone();
        assert!(config.validate().is_err());
    }
}

#[cfg(test)]
mod renewal_tests;
