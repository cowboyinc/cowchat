//! Checkpoint-batch publication through CBFS. One manifest commit publishes an
//! immutable segment and its discovery head together. This is not an ownership
//! allocator: callers must already hold the fenced owner-stream lease.
use super::cbqs::{
    unverified_archive_range, CbqsOwnerLog, LogError, LogRecord, Replay, VerifiedCheckpoint,
};
use cbfs_hooks::traits::{AuthoritativeStore, ManifestRegistry};
use cbfs_sdk::Volume;
use cbfs_types::{ManifestRoot, Visibility};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive requires a clean private volume and finite operation bounds")]
    Configuration,
    #[error("archive operation is uncertain; reopen from authoritative state")]
    Unavailable,
    #[error("archive head changed or batch does not extend the current head")]
    Conflict,
    #[error("archive discovery head or segment is missing")]
    HistoryGap,
    #[error("archive identity or segment bytes do not match")]
    Verification,
    #[error("archive object exceeds configured size bound")]
    Limit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveHead {
    version: u8,
    instance: [u8; 32],
    stream: [u8; 32],
    pub sequence: u64,
    pub checkpoint: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct ArchiveCommit {
    pub head: ArchiveHead,
    pub manifest_root: ManifestRoot,
}

#[derive(Clone, Copy)]
pub struct RecoveryLimits {
    pub max_segments: usize,
    pub max_records: u64,
    /// Total encoded bytes across the complete archive, not per segment.
    pub max_bytes: usize,
}

pub struct RecoveredArchive {
    pub records: Vec<LogRecord>,
    pub checkpoint: Option<VerifiedCheckpoint>,
}

fn verification_error(error: LogError) -> ArchiveError {
    match error {
        LogError::ReplayLimit => ArchiveError::Limit,
        _ => ArchiveError::Verification,
    }
}

pub struct CbfsArchive {
    volume: Volume,
    authority: Arc<dyn AuthoritativeStore>,
    registry: Arc<dyn ManifestRegistry>,
    instance: [u8; 32],
    stream: [u8; 32],
    prefix: String,
    head: Option<ArchiveHead>,
    max_segment_bytes: usize,
    timeout: Duration,
    usable: bool,
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl CbfsArchive {
    /// `volume` must have been opened against authenticated authority (or
    /// explicitly provisioned with a registered zero root). Missing authority
    /// is never interpreted as a new empty volume. Pending SDK journals are
    /// reconciled before loading the head or staging another batch.
    pub async fn open(
        volume: Volume,
        authority: Arc<dyn AuthoritativeStore>,
        registry: Arc<dyn ManifestRegistry>,
        instance: [u8; 32],
        stream: [u8; 32],
        max_segment_bytes: usize,
        timeout: Duration,
    ) -> Result<Self, ArchiveError> {
        if volume.visibility() != Visibility::Private
            || volume
                .has_staged_changes()
                .map_err(|_| ArchiveError::Unavailable)?
            || max_segment_bytes == 0
            || timeout.is_zero()
        {
            return Err(ArchiveError::Configuration);
        }
        let mut archive = Self {
            volume,
            authority,
            registry,
            instance,
            stream,
            prefix: format!("cowchat/{}/{}/", hex(&instance), hex(&stream)),
            head: None,
            max_segment_bytes,
            timeout,
            usable: true,
        };
        tokio::time::timeout(timeout, archive.load_head())
            .await
            .map_err(|_| ArchiveError::Unavailable)??;
        Ok(archive)
    }

    pub fn head(&self) -> Result<Option<&ArchiveHead>, ArchiveError> {
        self.ensure_usable()?;
        Ok(self.head.as_ref())
    }

    fn head_path(&self) -> String {
        format!("{}head.json", self.prefix)
    }
    fn segment_path(&self, checkpoint: &[u8; 32]) -> String {
        format!("{}{}.json", self.prefix, hex(checkpoint))
    }
    fn ensure_usable(&self) -> Result<(), ArchiveError> {
        if self.usable {
            Ok(())
        } else {
            Err(ArchiveError::Unavailable)
        }
    }

    async fn current_root(&self) -> Result<(), ArchiveError> {
        let root = self
            .authority
            .get_root(self.volume.volume_id())
            .await
            .map_err(|_| ArchiveError::Unavailable)?;
        match root {
            Some(root) if root == self.volume.manifest_root() => Ok(()),
            Some(_) => Err(ArchiveError::Conflict),
            None => Err(ArchiveError::Unavailable),
        }
    }

    async fn read_object(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ArchiveError> {
        let descriptor = self
            .volume
            .manifest()
            .get(path)
            .ok_or(ArchiveError::HistoryGap)?;
        if descriptor.plaintext_size(Visibility::Private) > max_bytes as u64 {
            return Err(ArchiveError::Limit);
        }
        let bytes = self
            .volume
            .get(path)
            .await
            .map_err(|_| ArchiveError::Unavailable)?;
        if bytes.len() > max_bytes {
            return Err(ArchiveError::Limit);
        }
        Ok(bytes)
    }

    async fn load_head(&mut self) -> Result<(), ArchiveError> {
        self.current_root().await?;
        // A previous commit may have landed without returning its receipt.
        // Resolve that journal before staging new bytes: recovery of an old
        // publication must not be mixed with a different batch's commit.
        self.volume
            .reconcile_pending_commit(self.authority.as_ref(), self.registry.as_ref())
            .await
            .map_err(|_| ArchiveError::Unavailable)?;
        self.current_root().await?;
        let path = self.head_path();
        if self.volume.manifest().get(&path).is_none() {
            if !self.volume.manifest().list_entries(&self.prefix).is_empty() {
                return Err(ArchiveError::HistoryGap);
            }
            return Ok(());
        }
        let bytes = self.read_object(&path, 4096).await?;
        let head: ArchiveHead =
            serde_json::from_slice(&bytes).map_err(|_| ArchiveError::Verification)?;
        if head.version != 1
            || head.instance != self.instance
            || head.stream != self.stream
            || head.sequence == 0
            || self
                .volume
                .manifest()
                .get(&self.segment_path(&head.checkpoint))
                .is_none()
        {
            return Err(ArchiveError::Verification);
        }
        self.head = Some(head);
        Ok(())
    }

    /// Bytes must still go through `CbqsOwnerLog::restore_archive`; a CBFS head
    /// is authenticated discovery, not a replacement for CBQS proof checking.
    pub async fn read_segment(&self, checkpoint: &[u8; 32]) -> Result<Vec<u8>, ArchiveError> {
        self.ensure_usable()?;
        tokio::time::timeout(
            self.timeout,
            self.read_object(&self.segment_path(checkpoint), self.max_segment_bytes),
        )
        .await
        .map_err(|_| ArchiveError::Unavailable)?
    }

    /// Cold recovery starts only at the authenticated CBFS head. Receipt links
    /// guide bounded backward discovery; every segment is then verified FORWARD
    /// from genesis. No partial records escape on a missing/forged segment or a
    /// changed root. The complete recovery shares this writer's timeout.
    pub async fn recover(
        &mut self,
        log: &mut CbqsOwnerLog,
        limits: RecoveryLimits,
    ) -> Result<RecoveredArchive, ArchiveError> {
        self.ensure_usable()?;
        self.usable = false;
        let result = tokio::time::timeout(self.timeout, self.recover_inner(log, limits))
            .await
            .map_err(|_| ArchiveError::Unavailable)
            .and_then(|result| result);
        if result.is_ok() {
            self.usable = true;
        }
        result
    }

    async fn recover_inner(
        &self,
        log: &mut CbqsOwnerLog,
        limits: RecoveryLimits,
    ) -> Result<RecoveredArchive, ArchiveError> {
        if limits.max_segments == 0 || limits.max_records == 0 || limits.max_bytes == 0 {
            return Err(ArchiveError::Configuration);
        }
        if log.archive_identity() != (self.instance, self.stream) {
            return Err(ArchiveError::Verification);
        }
        self.current_root().await?;
        let mut recovered = RecoveredArchive {
            records: Vec::new(),
            checkpoint: None,
        };
        let Some(head) = &self.head else {
            return Ok(recovered);
        };
        if head.sequence > limits.max_records {
            return Err(ArchiveError::Limit);
        }
        let genesis =
            cowboy_protocol_codec::cbqs_v2::checkpoint_genesis_v2(&self.instance, &self.stream);
        let (mut checkpoint, mut sequence) = (head.checkpoint, head.sequence);
        let mut remaining_bytes = limits.max_bytes;
        let mut segments = Vec::new();
        while checkpoint != genesis {
            if segments.len() == limits.max_segments {
                return Err(ArchiveError::Limit);
            }
            let bytes = self
                .read_object(
                    &self.segment_path(&checkpoint),
                    self.max_segment_bytes.min(remaining_bytes),
                )
                .await?;
            remaining_bytes = remaining_bytes
                .checked_sub(bytes.len())
                .ok_or(ArchiveError::Limit)?;
            let range = unverified_archive_range(&bytes, limits.max_records, bytes.len())
                .map_err(verification_error)?;
            if range.instance != self.instance
                || range.stream != self.stream
                || range.checkpoint != checkpoint
                || range.last != sequence
                || range.first == 0
                || range.first > range.last
            {
                return Err(ArchiveError::Verification);
            }
            // Strictly decreasing sequence bounds also reject cycles, without
            // another unbounded set of checkpoint IDs.
            sequence = range.first - 1;
            checkpoint = range.previous;
            segments.push(bytes);
        }
        if sequence != 0 {
            return Err(ArchiveError::HistoryGap);
        }
        for bytes in segments.into_iter().rev() {
            let remaining_records = limits
                .max_records
                .checked_sub(recovered.records.len() as u64)
                .ok_or(ArchiveError::Limit)?;
            let replay = log
                .restore_archive(
                    &bytes,
                    recovered.checkpoint.as_ref(),
                    remaining_records,
                    bytes.len(),
                )
                .map_err(verification_error)?;
            recovered.records.extend(replay.records);
            recovered.checkpoint = replay.checkpoint;
        }
        if recovered
            .checkpoint
            .as_ref()
            .map(|c| (c.sequence(), c.id()))
            != Some((head.sequence, head.checkpoint))
        {
            return Err(ArchiveError::Verification);
        }
        self.current_root().await?;
        Ok(recovered)
    }

    /// Publish a complete verified range with one CBFS manifest commit. A
    /// repeated range must already be byte-identical; no rewrite or fork is
    /// allowed. Any error or cancellation retires this instance, including an
    /// unknown commit outcome. Reopen/reconcile instead of blindly retrying.
    pub async fn publish(&mut self, replay: &Replay) -> Result<ArchiveCommit, ArchiveError> {
        self.ensure_usable()?;
        // Set before the first await, so dropping this future also retires it.
        self.usable = false;
        let result = tokio::time::timeout(self.timeout, self.publish_inner(replay))
            .await
            .map_err(|_| ArchiveError::Unavailable)
            .and_then(|result| result);
        if result.is_ok() {
            self.usable = true;
        }
        result
    }

    async fn publish_inner(&mut self, replay: &Replay) -> Result<ArchiveCommit, ArchiveError> {
        let range = replay.archive_range().ok_or(ArchiveError::Configuration)?;
        if range.instance != self.instance || range.stream != self.stream {
            return Err(ArchiveError::Verification);
        }
        let bytes = replay
            .archive_bytes(self.max_segment_bytes)
            .map_err(|_| ArchiveError::Limit)?
            .ok_or(ArchiveError::Configuration)?;
        self.current_root().await?;
        let path = self.segment_path(&range.checkpoint);
        let exists = self.volume.manifest().get(&path).is_some();
        if exists && self.read_object(&path, self.max_segment_bytes).await? != bytes {
            return Err(ArchiveError::Verification);
        }
        if self
            .head
            .as_ref()
            .is_some_and(|head| head.checkpoint == range.checkpoint && head.sequence == range.last)
        {
            if !exists {
                return Err(ArchiveError::HistoryGap);
            }
            return Ok(ArchiveCommit {
                head: self.head.clone().unwrap(),
                manifest_root: self.volume.manifest_root(),
            });
        }
        let (previous, sequence) = self.head.as_ref().map_or_else(
            || {
                (
                    cowboy_protocol_codec::cbqs_v2::checkpoint_genesis_v2(
                        &self.instance,
                        &self.stream,
                    ),
                    0,
                )
            },
            |head| (head.checkpoint, head.sequence),
        );
        if previous != range.previous || sequence.checked_add(1) != Some(range.first) {
            return Err(ArchiveError::Conflict);
        }
        // One manifest publication makes both objects discoverable atomically.
        // CBFS's prev-root CAS rejects a concurrent archive-head overwrite.
        if !exists {
            self.volume
                .put(&path, &bytes)
                .await
                .map_err(|_| ArchiveError::Unavailable)?;
        }
        let head = ArchiveHead {
            version: 1,
            instance: self.instance,
            stream: self.stream,
            sequence: range.last,
            checkpoint: range.checkpoint,
        };
        let head_bytes = serde_json::to_vec(&head).map_err(|_| ArchiveError::Verification)?;
        self.volume
            .put(&self.head_path(), &head_bytes)
            .await
            .map_err(|_| ArchiveError::Unavailable)?;
        let receipt = self
            .volume
            .commit(self.authority.as_ref(), self.registry.as_ref())
            .await
            .map_err(|_| ArchiveError::Unavailable)?;
        self.current_root().await?;
        self.head = Some(head.clone());
        Ok(ArchiveCommit {
            head,
            manifest_root: receipt.manifest_root,
        })
    }
}
