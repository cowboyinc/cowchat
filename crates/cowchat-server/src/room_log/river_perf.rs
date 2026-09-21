//! Explicit paid River probe. CBQS uses the local real-broker fixture; CBFS
//! uses ONLY Cowboy chain-backed authority. No production service is deployed.
use super::*;
use anyhow::{ensure, Context, Result};
use cbfs_cli::{
    authenticated_state::TrustedCheckpoint,
    config::CliMode,
    connect::{create_volume, open_volume, VolumeContext},
};
use cbfs_registry_proto::AccessMode;
use serde::Deserialize;
use std::{fs::File, io::Write, os::unix::fs::OpenOptionsExt, time::Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    cbfs_state_dir: PathBuf,
    rpc_url: String,
    trusted_checkpoint_file: PathBuf,
    erasure_k: u8,
    erasure_m: u8,
    /// Decimal wei, explicitly not CBY or a floating point JSON number.
    initial_reserve_wei: String,
    samples: usize,
    promotions: usize,
    output_file: PathBuf,
}

impl Config {
    fn validate(&self) -> Result<u128> {
        ensure!((3..=100).contains(&self.samples), "samples must be 3..=100");
        ensure!(
            (1..=20).contains(&self.promotions),
            "promotions must be 1..=20"
        );
        ensure!(
            self.erasure_k > 0 && self.erasure_m > 0,
            "private erasure layout requires data and parity"
        );
        ensure!(
            u16::from(self.erasure_k) + u16::from(self.erasure_m) <= 255,
            "invalid erasure layout"
        );
        for path in [
            &self.cbfs_state_dir,
            &self.trusted_checkpoint_file,
            &self.output_file,
        ] {
            ensure!(path.is_absolute(), "configuration paths must be absolute");
        }
        let url = reqwest::Url::parse(&self.rpc_url)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "RPC must be an HTTP base URL without credentials/query/fragment"
        );
        let reserve = self
            .initial_reserve_wei
            .parse::<u128>()
            .context("invalid initial_reserve_wei")?;
        ensure!(reserve > 0, "a funded volume reserve is required");
        Ok(reserve)
    }
}

fn record(output: &mut File, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *output, &value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn distribution(values: &[f64]) -> serde_json::Value {
    assert!(!values.is_empty());
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let percentile = |p: usize| ordered[(p * ordered.len()).div_ceil(100) - 1];
    serde_json::json!({"n":values.len(),"p50_ms":percentile(50),"p95_ms":percentile(95),"p99_ms":percentile(99),"min_ms":ordered[0],"max_ms":ordered[ordered.len()-1]})
}

async fn new_volume(
    config: &Config,
    checkpoint: &TrustedCheckpoint,
    name: &str,
    reserve: u128,
) -> Result<VolumeContext> {
    // Only a successful NEW registration allows genesis initialization below.
    // Never reopen an arbitrary empty volume and infer freshness from its root.
    let created = create_volume(
        &config.cbfs_state_dir,
        name,
        &[],
        config.erasure_k,
        config.erasure_m,
        Visibility::Private,
        CliMode::Cowboy,
        Some(&config.rpc_url),
        reserve,
        true,
        Some(checkpoint),
    )
    .await?;
    let opened = open_volume(
        &config.cbfs_state_dir,
        name,
        CliMode::Cowboy,
        Some(&config.rpc_url),
        AccessMode::ReadWrite,
        false,
        0,
        Some(checkpoint),
    )
    .await?;
    ensure!(
        opened.http_config.is_some(),
        "refusing a non-Cowboy authority"
    );
    ensure!(
        opened.volume.volume_id().0 == created.volume_id,
        "new volume identity changed on open"
    );
    Ok(opened)
}

async fn probe(config: &Config, reserve: u128, output: &mut File) -> Result<()> {
    let checkpoint = TrustedCheckpoint::load(&config.trusted_checkpoint_file)?;
    let run = uuid::Uuid::new_v4();
    let control_name = format!("cowchat-perf-{run}-control");
    let archive_name = format!("cowchat-perf-{run}-archive");
    record(
        output,
        serde_json::json!({"event":"started","run":run,"topology":"local real CBQS with fixture chain authority; real River chain-backed CBFS; local authenticated TCP Cowchat clients",
        "control_volume_name":control_name,"archive_volume_name":archive_name,"samples":config.samples,"promotions":config.promotions,
        "erasure_k":config.erasure_k,"erasure_m":config.erasure_m,"initial_reserve_wei_per_volume":config.initial_reserve_wei,
        "cbfs_pin":"d8ddaad0dee6a0b8b57bd7308376208446ced012","cbqs_pin":"7f0fb216bfa2dc53d618224aecf3c7cb628a3015",
        "debug_assertions":cfg!(debug_assertions),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "checkpoint_height":checkpoint.height(),"client_request_timeout_ms":10000,"payload_plaintext_bytes":1024,"batch_messages":1,"warmup_samples":0}),
    )?;
    cbfs_sdk::test_support::isolate_state_dirs();
    let fixture = Fixture::new().await;
    let timeout = Duration::from_secs(120);
    record(
        output,
        serde_json::json!({"event":"phase","name":"register_control"}),
    )?;
    let control = new_volume(config, &checkpoint, &control_name, reserve).await?;
    let control_id = control.volume.volume_id().0;
    let mut writers = WriterRegistry::initialize_new_volume(
        control.volume,
        control.auth_store,
        control.registry,
        INSTANCE,
        STREAM,
        timeout,
    )
    .await?;
    record(
        output,
        serde_json::json!({"event":"phase","name":"register_archive"}),
    )?;
    let archive = new_volume(config, &checkpoint, &archive_name, reserve).await?;
    ensure!(
        archive.volume.volume_id().0 != control_id,
        "control/archive volumes must differ"
    );
    let archive = CbfsArchive::initialize_new_volume(
        archive.volume,
        archive.auth_store,
        archive.registry,
        INSTANCE,
        STREAM,
        MAX_BYTES,
        timeout,
    )
    .await?;
    let mut promotion_ms = Vec::new();
    let mut writer = None;
    for sample in 0..config.promotions {
        record(
            output,
            serde_json::json!({"event":"phase","name":"promotion","sample":sample}),
        )?;
        let worker = WorkerIncarnation::fresh();
        let expected = writers.epoch()?;
        let start = Instant::now();
        let allocation = writers
            .claim(expected, worker.writer_id(), worker.claim_id())
            .await?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        promotion_ms.push(elapsed);
        let fence_start = Instant::now();
        let log = fixture
            .connect_config(super::super::config(&fixture, &worker, allocation.epoch()))
            .await?;
        writer = Some(worker.bind(allocation, log)?);
        record(
            output,
            serde_json::json!({"event":"promotion","sample":sample,"control_cas_ms":elapsed,"local_fence_session_ms":fence_start.elapsed().as_secs_f64()*1000.0}),
        )?;
    }
    let directory = private_directory();
    // Large enough for this bounded probe, never unbounded production history.
    let journal = journal(&directory.path().join("intents.sqlite"));
    let runtime = OwnerRuntime::recover(
        "owner-a".into(),
        writer.context("missing fenced writer")?,
        archive,
        journal,
        RuntimeLimits {
            archive: RecoveryLimits {
                max_segments: 1000,
                max_records: 1000,
                max_bytes: 64 * 1024 * 1024,
            },
            batch_records: 10,
            replay_bytes: MAX_BYTES,
        },
    )
    .await?;
    let network = Network::new(runtime).await;
    let alice = network.client("river-perf-alice").await;
    let bob = network.client("river-perf-bob").await;
    record(
        output,
        serde_json::json!({"event":"phase","name":"create_room"}),
    )?;
    let prepared = CowchatClient::prepare_hosted_room("river-perf");
    let start = Instant::now();
    let room = alice.create_prepared_room(&prepared).await?;
    record(
        output,
        serde_json::json!({"event":"create_room","ack_ms":start.elapsed().as_secs_f64()*1000.0}),
    )?;
    alice.join_room(&room.room_id).await?;
    bob.join_room(&room.room_id).await?;
    let mut ack_ms = Vec::new();
    let body = "x".repeat(1024);
    for sample in 0..config.samples {
        // Prepare outside the timed interval; the sample starts at client send
        // and ends on its actual archive-gated reply. No retries hide failures.
        let message =
            alice.prepare_message(&room.room_id, &body, None, vec![], serde_json::json!({}));
        record(
            output,
            serde_json::json!({"event":"phase","name":"send","sample":sample}),
        )?;
        let start = Instant::now();
        let receipt = alice.append_prepared_message(&message).await?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        ensure!(
            receipt.seq == (sample + 1) as i64,
            "unexpected acknowledged sequence"
        );
        // The independently authenticated reader must observe the acknowledged
        // result; read cost is intentionally outside ACK timing.
        let history = bob
            .get_history_filtered(&room.room_id, 1, None, None, Some(sample as i64))
            .await?;
        ensure!(
            history.len() == 1
                && history[0].message_id == receipt.message_id
                && history[0].content == body,
            "acknowledged message missing or changed in participant history"
        );
        let duplicate = alice.append_prepared_message(&message).await?;
        ensure!(
            duplicate.message_id == receipt.message_id && duplicate.seq == receipt.seq,
            "retry receipt changed"
        );
        ack_ms.push(elapsed);
        record(
            output,
            serde_json::json!({"event":"send","sample":sample,"archive_gated_client_ack_ms":elapsed,"sequence":receipt.seq}),
        )?;
    }
    ensure!(
        bob.get_history(&room.room_id, 1000, None).await?.len() == config.samples,
        "duplicate messages after stable retries"
    );
    record(
        output,
        serde_json::json!({"event":"complete","archive_gated_client_ack":distribution(&ack_ms),"control_volume_cas":distribution(&promotion_ms),
        "percentile_method":"nearest-rank; all samples, no warmup discarded","scope":"single-message batches, local CBQS fixture authority, River CBFS Stage/Finalize; not deployed CBQS authority or cross-host failover",
        "cleanup":"probe volumes retained on disposable River; tear down the dedicated box after collecting results"}),
    )?;
    Ok(())
}

#[tokio::test]
#[ignore = "creates and funds private volumes on River; requires explicit COWCHAT_RIVER_PERF_CONFIG"]
async fn river_archive_perf() -> Result<()> {
    let path = std::env::var_os("COWCHAT_RIVER_PERF_CONFIG")
        .context("set COWCHAT_RIVER_PERF_CONFIG to the explicit River config JSON")?;
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let reserve = config.validate()?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&config.output_file)?;
    let result = probe(&config, reserve, &mut output).await;
    if result.is_err() {
        // No success-only percentile summary for a failed/truncated run. Error
        // details go to the test runner, never keys/config into the artifact.
        record(
            &mut output,
            serde_json::json!({"event":"failed","complete":false}),
        )?;
    }
    result
}

#[test]
fn nearest_rank_keeps_slow_tail_and_single_promotion() {
    let values: Vec<_> = (1..=100).rev().map(f64::from).collect();
    let summary = distribution(&values);
    assert_eq!(summary["p50_ms"], 50.0);
    assert_eq!(summary["p95_ms"], 95.0);
    assert_eq!(summary["p99_ms"], 99.0);
    assert_eq!(distribution(&[42.0])["p99_ms"], 42.0);
}
