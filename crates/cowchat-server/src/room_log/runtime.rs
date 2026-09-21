//! Serialized owner execution. Neither replies nor projection reads are
//! available while a batch is uncertain. Callers put this behind an async
//! mutex/queue; no synchronous room lock may be held across these operations.
use super::{
    cbfs_archive::{ArchiveError, CbfsArchive, RecoveryLimits},
    cbqs::{CbqsOwnerLog, LogError, LogRecord, VerifiedCheckpoint},
    intent::{Intent, IntentError, IntentJournal},
    ownership::AllocatedWriter,
    Command, CommandBody, Outcome, OwnerState, ReplayError,
};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("owner runtime identity, volume or bounds configuration is invalid")]
    Configuration,
    #[error("owner runtime is retired; recover under a newly fenced writer")]
    Retired,
    #[error("owner projection capacity exhausted")]
    Limit,
    #[error(transparent)]
    Log(#[from] LogError),
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error(transparent)]
    Intent(#[from] IntentError),
    #[error(transparent)]
    Replay(#[from] ReplayError),
}

/// Fresh per process incarnation, not a hostname or deserializable identity.
/// Keep this value while retrying one uncertain claim. A restarted worker
/// creates a new incarnation and must acquire a new epoch through promotion.
pub struct WorkerIncarnation {
    holder: SigningKey,
    writer_id: String,
    claim_id: String,
}
impl WorkerIncarnation {
    pub fn fresh() -> Self {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let holder = SigningKey::from_bytes(&secret);
        let nonce = uuid::Uuid::new_v4();
        let mut digest = Sha256::new();
        digest.update(b"cowchat/writer-incarnation/1\0");
        digest.update(nonce.as_bytes());
        digest.update(holder.verifying_key().to_bytes());
        let writer_id = digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Self {
            holder,
            writer_id,
            claim_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    /// For an authenticated grant issuer/SessionConfig, never for log encoding.
    pub fn holder(&self) -> &SigningKey {
        &self.holder
    }
    pub fn writer_id(&self) -> &str {
        &self.writer_id
    }
    pub fn claim_id(&self) -> &str {
        &self.claim_id
    }

    /// The log can only be constructed by its matching signed-fence handshake.
    /// Consume the incarnation so it cannot accidentally start two runtimes.
    pub fn bind(
        self,
        allocation: AllocatedWriter,
        log: CbqsOwnerLog,
    ) -> Result<FencedWriter, RuntimeError> {
        if allocation.writer_id() != self.writer_id
            || allocation.claim_id() != self.claim_id
            || allocation.epoch() != log.epoch()
            || allocation.stream_identity() != log.archive_identity()
            || log.holder_key() != self.holder.verifying_key().to_bytes()
        {
            return Err(RuntimeError::Configuration);
        }
        Ok(FencedWriter { allocation, log })
    }
}

pub struct FencedWriter {
    allocation: AllocatedWriter,
    log: CbqsOwnerLog,
}

#[derive(Clone, Copy)]
pub struct RuntimeLimits {
    /// Until compaction exists, this also caps the entire in-memory projection.
    pub archive: RecoveryLimits,
    pub batch_records: usize,
    pub replay_bytes: usize,
}

pub struct CommittedBatch {
    /// One receipt per submitted command, including retries/business rejection.
    pub outcomes: Vec<Outcome>,
    /// Only first applications, for post-commit fanout. Retries do not emit
    /// duplicate events. These records have passed the archive durability gate.
    pub applied: Vec<LogRecord>,
}

pub struct OwnerRuntime {
    owner_id: String,
    log: CbqsOwnerLog,
    archive: CbfsArchive,
    journal: IntentJournal,
    state: OwnerState,
    checkpoint: Option<VerifiedCheckpoint>,
    limits: RuntimeLimits,
    segments: usize,
    encoded_bytes: usize,
    usable: bool,
}

impl OwnerRuntime {
    pub async fn recover(
        owner_id: String,
        writer: FencedWriter,
        mut archive: CbfsArchive,
        journal: IntentJournal,
        limits: RuntimeLimits,
    ) -> Result<Self, RuntimeError> {
        if !super::valid_identity(&owner_id)
            || writer.allocation.control_volume() == archive.volume_id()
            || !journal.matches(&owner_id, writer.log.archive_identity())
            || limits.batch_records == 0
            || limits.replay_bytes == 0
            || limits.archive.max_records == 0
            || limits.archive.max_bytes == 0
            || limits.archive.max_segments == 0
        {
            return Err(RuntimeError::Configuration);
        }
        let mut log = writer.log;
        let recovered = archive.recover(&mut log, limits.archive).await?;
        let mut state = OwnerState::new(owner_id.clone());
        for record in recovered.records {
            state.apply(record.sequence, record.lane_id, &record.command)?;
        }
        let mut runtime = Self {
            owner_id,
            log,
            archive,
            journal,
            state,
            checkpoint: recovered.checkpoint,
            limits,
            segments: recovered.segments,
            encoded_bytes: recovered.encoded_bytes,
            usable: false,
        };
        // Archive any surviving but previously unacknowledged broker suffix
        // before it becomes served state. Never bridge an expired/missing gap.
        runtime
            .capture_suffix(runtime.state.applied_through())
            .await?;
        let pending = runtime.journal.pending().await?;
        runtime.finish_pending(&pending).await?;
        runtime.journal.clear().await?;
        runtime.usable = true;
        Ok(runtime)
    }

    pub fn state(&self) -> Result<&OwnerState, RuntimeError> {
        self.ensure_usable()?;
        Ok(&self.state)
    }

    fn ensure_usable(&self) -> Result<(), RuntimeError> {
        if self.usable {
            Ok(())
        } else {
            Err(RuntimeError::Retired)
        }
    }

    /// Authenticated ingress supplies stable IDs/ciphertext. CreateRoom's
    /// lane_id is assigned here through the stable owner/room allocation key.
    /// Keep the prepared commands at the caller until this returns a receipt.
    pub async fn submit(
        &mut self,
        mut commands: Vec<Command>,
    ) -> Result<CommittedBatch, RuntimeError> {
        self.ensure_usable()?;
        if commands.is_empty() || commands.len() > self.limits.batch_records {
            return Err(RuntimeError::Limit);
        }
        if commands
            .iter()
            .any(|c| c.owner_id != self.owner_id || !super::valid_identity(&c.command_id))
        {
            return Err(RuntimeError::Configuration);
        }
        if commands.iter().any(|c| matches!(&c.body,
            CommandBody::AppendMessage { ciphertext, .. } if !cowchat_core::crypto::is_ciphertext(ciphertext))) {
            return Err(RuntimeError::Configuration);
        }
        // Set before the first await. Error or future cancellation leaves the
        // complete owner inaccessible, including history and subsequent sends.
        self.usable = false;
        let result = self.submit_inner(&mut commands).await;
        if result.is_ok() {
            self.usable = true;
        }
        result
    }

    async fn submit_inner(
        &mut self,
        commands: &mut [Command],
    ) -> Result<CommittedBatch, RuntimeError> {
        let mut intents = Vec::with_capacity(commands.len());
        for command in commands {
            let lane_id = match &mut command.body {
                CommandBody::CreateRoom {
                    room_id, lane_id, ..
                } => {
                    *lane_id = self.log.room_lane(&self.owner_id, room_id).await?;
                    0
                }
                CommandBody::AppendMessage { room_id, .. } => {
                    self.log.room_lane(&self.owner_id, room_id).await?
                }
            };
            intents.push(Intent {
                lane_id,
                command: command.clone(),
            });
        }
        // One local transaction for the whole prepared batch, before appends.
        self.journal.stage(&intents).await?;
        let result = self.finish_pending(&intents).await?;
        self.journal.clear().await?;
        Ok(result)
    }

    async fn finish_pending(&mut self, intents: &[Intent]) -> Result<CommittedBatch, RuntimeError> {
        if intents.len() > self.limits.batch_records {
            return Err(RuntimeError::Limit);
        }
        let mut minimum = self.state.applied_through();
        let mut remaining = self.limits.archive.max_records.saturating_sub(minimum);
        for intent in intents {
            if self.state.receipt(&intent.command)?.is_some() {
                continue;
            }
            if remaining == 0 {
                return Err(RuntimeError::Limit);
            }
            remaining -= 1;
            minimum = self.log.append(intent.lane_id, &intent.command).await?;
        }
        let applied = if minimum > self.state.applied_through() {
            self.capture_suffix(minimum).await?
        } else {
            Vec::new()
        };
        let outcomes = intents
            .iter()
            .map(|intent| {
                self.state
                    .receipt(&intent.command)?
                    .ok_or(RuntimeError::Configuration)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CommittedBatch { outcomes, applied })
    }

    async fn capture_suffix(&mut self, minimum: u64) -> Result<Vec<LogRecord>, RuntimeError> {
        let replay = self
            .log
            .replay(
                self.checkpoint.as_ref(),
                minimum,
                self.limits.archive.max_records,
                self.limits.replay_bytes,
            )
            .await?;
        if replay.records.is_empty() {
            return Ok(Vec::new());
        }
        if self
            .state
            .applied_through()
            .checked_add(replay.records.len() as u64)
            .is_none_or(|n| n > self.limits.archive.max_records)
        {
            return Err(RuntimeError::Limit);
        }
        // No state mutation (nor returned fanout record) precedes this commit.
        if self.segments >= self.limits.archive.max_segments {
            return Err(RuntimeError::Limit);
        }
        let bytes = replay
            .archive_bytes(
                self.limits
                    .archive
                    .max_bytes
                    .saturating_sub(self.encoded_bytes),
            )?
            .ok_or(RuntimeError::Configuration)?
            .len();
        self.archive.publish(&replay).await?;
        self.segments += 1;
        self.encoded_bytes += bytes;
        let mut applied = Vec::new();
        for record in replay.records {
            let first = self.state.receipt(&record.command)?.is_none();
            let outcome = self
                .state
                .apply(record.sequence, record.lane_id, &record.command)?;
            if first && !matches!(outcome, Outcome::Rejected { .. }) {
                applied.push(record);
            }
        }
        self.checkpoint = replay.checkpoint;
        Ok(applied)
    }
}
