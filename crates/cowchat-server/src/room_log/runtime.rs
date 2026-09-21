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
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock, RwLockReadGuard,
};

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

/// The single committed projection shared by synchronous authorization reads.
/// A pending write leaves previous committed state readable; uncertainty retires
/// the view. Never hold its guard across an await or a broker mutation.
pub struct OwnerView {
    state: RwLock<OwnerState>,
    usable: AtomicBool,
}
impl OwnerView {
    pub fn read(&self) -> Result<RwLockReadGuard<'_, OwnerState>, RuntimeError> {
        let state = self.state.read().map_err(|_| RuntimeError::Retired)?;
        if !self.usable.load(Ordering::Acquire) {
            return Err(RuntimeError::Retired);
        }
        Ok(state)
    }
    fn apply(&self, records: Vec<LogRecord>) -> Result<Vec<LogRecord>, RuntimeError> {
        let mut state = self.state.write().map_err(|_| RuntimeError::Retired)?;
        let result = (|| {
            let mut applied = Vec::new();
            for record in records {
                let first = state.receipt(&record.command)?.is_none();
                let outcome = state.apply(record.sequence, record.lane_id, &record.command)?;
                if first && !matches!(outcome, Outcome::Rejected { .. }) {
                    applied.push(record);
                }
            }
            Ok(applied)
        })();
        // Mark failure BEFORE releasing the write guard, so another reader
        // can never observe a partially applied batch from a failed reducer.
        if result.is_err() {
            self.usable.store(false, Ordering::Release);
        }
        result
    }
}
struct RetireView {
    view: Arc<OwnerView>,
    completed: bool,
}
impl Drop for RetireView {
    fn drop(&mut self) {
        if !self.completed {
            self.view.usable.store(false, Ordering::Release);
        }
    }
}

pub struct OwnerRuntime {
    owner_id: String,
    log: CbqsOwnerLog,
    archive: CbfsArchive,
    journal: IntentJournal,
    view: Arc<OwnerView>,
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
            view: Arc::new(OwnerView {
                state: RwLock::new(state),
                usable: AtomicBool::new(false),
            }),
            checkpoint: recovered.checkpoint,
            limits,
            segments: recovered.segments,
            encoded_bytes: recovered.encoded_bytes,
            usable: false,
        };
        // Archive any surviving but previously unacknowledged broker suffix
        // before it becomes served state. Never bridge an expired/missing gap.
        let minimum = runtime.read_state()?.applied_through();
        runtime.capture_suffix(minimum).await?;
        let pending = runtime.journal.pending().await?;
        runtime.finish_pending(&pending).await?;
        runtime.journal.clear().await?;
        runtime.usable = true;
        runtime.view.usable.store(true, Ordering::Release);
        Ok(runtime)
    }

    pub fn state(&self) -> Result<RwLockReadGuard<'_, OwnerState>, RuntimeError> {
        self.ensure_usable()?;
        self.view.read()
    }
    pub fn view(&self) -> Arc<OwnerView> {
        Arc::clone(&self.view)
    }
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }
    fn read_state(&self) -> Result<RwLockReadGuard<'_, OwnerState>, RuntimeError> {
        self.view.state.read().map_err(|_| RuntimeError::Retired)
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
        let mut retirement = RetireView {
            view: self.view(),
            completed: false,
        };
        let result = self.submit_inner(&mut commands).await;
        if result.is_ok() {
            self.usable = true;
            retirement.completed = true;
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
        let mut minimum = self.read_state()?.applied_through();
        let mut remaining = self.limits.archive.max_records.saturating_sub(minimum);
        for intent in intents {
            if self.read_state()?.receipt(&intent.command)?.is_some() {
                continue;
            }
            if remaining == 0 {
                return Err(RuntimeError::Limit);
            }
            remaining -= 1;
            minimum = self.log.append(intent.lane_id, &intent.command).await?;
        }
        let applied = if minimum > self.read_state()?.applied_through() {
            self.capture_suffix(minimum).await?
        } else {
            Vec::new()
        };
        let outcomes = intents
            .iter()
            .map(|intent| {
                self.read_state()?
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
            .read_state()?
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
        let applied = self.view.apply(replay.records)?;
        self.checkpoint = replay.checkpoint;
        Ok(applied)
    }
}

impl Drop for OwnerRuntime {
    fn drop(&mut self) {
        self.view.usable.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_reducer_batch_retires_the_shared_projection() {
        let view = OwnerView {
            state: RwLock::new(OwnerState::new("owner-a".into())),
            usable: AtomicBool::new(true),
        };
        let mut wrong_owner = super::super::tests::message("wrong", "one");
        wrong_owner.owner_id = "owner-b".into();
        assert!(view
            .apply(vec![
                LogRecord {
                    sequence: 1,
                    lane_id: 0,
                    command: super::super::tests::create("one", 7)
                },
                LogRecord {
                    sequence: 2,
                    lane_id: 7,
                    command: wrong_owner
                },
            ])
            .is_err());
        assert!(matches!(view.read(), Err(RuntimeError::Retired)));
    }
}
