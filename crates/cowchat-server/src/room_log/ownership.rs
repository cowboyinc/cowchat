//! Rare writer promotion through a dedicated private CBFS control volume.
//! Allocation is NOT permission to append: the caller must obtain a matching
//! CBQS fence ACK before recovery/serving. Failure detection is caller policy;
//! a losing candidate must not automatically compete for the next epoch.
use cbfs_hooks::traits::{AuthoritativeStore, ManifestRegistry};
use cbfs_sdk::Volume;
use cbfs_types::{ManifestRoot, Visibility, VolumeId};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

const MAX_RECORD_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum OwnershipError {
    #[error("ownership requires a clean private control volume and finite timeout")]
    Configuration,
    #[error("ownership outcome is uncertain; reopen and reconcile the same claim")]
    Unavailable,
    #[error("another writer or root superseded the expected ownership state")]
    Conflict,
    #[error("ownership record or claim identity is invalid")]
    InvalidRecord,
    #[error("finite writer epochs are exhausted")]
    EpochExhausted,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    instance: [u8; 32],
    stream: [u8; 32],
    epoch: u64,
    writer_id: String,
    claim_id: String,
}

/// Produced only by a successful authoritative claim, never deserialized.
/// This still does not grant a CBQS session or bypass its fence.
pub struct AllocatedWriter {
    record: Record,
    control_volume: VolumeId,
    root: ManifestRoot,
}
impl AllocatedWriter {
    pub fn epoch(&self) -> u64 {
        self.record.epoch
    }
    pub fn writer_id(&self) -> &str {
        &self.record.writer_id
    }
    pub fn claim_id(&self) -> &str {
        &self.record.claim_id
    }
    pub fn stream_identity(&self) -> ([u8; 32], [u8; 32]) {
        (self.record.instance, self.record.stream)
    }
    pub fn control_volume(&self) -> VolumeId {
        self.control_volume
    }
    pub fn root(&self) -> ManifestRoot {
        self.root
    }
}

pub struct WriterRegistry {
    volume: Volume,
    authority: Arc<dyn AuthoritativeStore>,
    registry: Arc<dyn ManifestRegistry>,
    instance: [u8; 32],
    stream: [u8; 32],
    path: String,
    record: Option<Record>,
    timeout: Duration,
    usable: bool,
}

impl WriterRegistry {
    /// Open from authenticated authority. The runtime must configure a volume
    /// distinct from its archive volume so promotion has an independent CAS.
    pub async fn open(
        volume: Volume,
        authority: Arc<dyn AuthoritativeStore>,
        registry: Arc<dyn ManifestRegistry>,
        instance: [u8; 32],
        stream: [u8; 32],
        timeout: Duration,
    ) -> Result<Self, OwnershipError> {
        Self::open_inner(
            volume, authority, registry, instance, stream, timeout, false,
        )
        .await
    }

    /// Explicit INITIAL PROVISIONING of a newly registered control volume.
    /// Never call this during worker startup, failover or missing-record repair:
    /// an empty root does not prove the volume has never been used. The volume
    /// provisioning workflow must establish that this is a new volume ID.
    pub async fn initialize_new_volume(
        volume: Volume,
        authority: Arc<dyn AuthoritativeStore>,
        registry: Arc<dyn ManifestRegistry>,
        instance: [u8; 32],
        stream: [u8; 32],
        timeout: Duration,
    ) -> Result<Self, OwnershipError> {
        Self::open_inner(volume, authority, registry, instance, stream, timeout, true).await
    }

    async fn open_inner(
        volume: Volume,
        authority: Arc<dyn AuthoritativeStore>,
        registry: Arc<dyn ManifestRegistry>,
        instance: [u8; 32],
        stream: [u8; 32],
        timeout: Duration,
        initialize: bool,
    ) -> Result<Self, OwnershipError> {
        if volume.visibility() != Visibility::Private
            || timeout.is_zero()
            || volume
                .has_staged_changes()
                .map_err(|_| OwnershipError::Unavailable)?
            || !volume.manifest().list_entries("cowchat/").is_empty()
        {
            return Err(OwnershipError::Configuration);
        }
        let hex = |bytes: &[u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut writer = Self {
            volume,
            authority,
            registry,
            instance,
            stream,
            path: format!("cowchat-owners/{}/{}.json", hex(&instance), hex(&stream)),
            record: None,
            timeout,
            usable: true,
        };
        tokio::time::timeout(timeout, writer.load(initialize))
            .await
            .map_err(|_| OwnershipError::Unavailable)??;
        Ok(writer)
    }

    async fn current_root(&self) -> Result<(), OwnershipError> {
        match self
            .authority
            .get_root(self.volume.volume_id())
            .await
            .map_err(|_| OwnershipError::Unavailable)?
        {
            Some(root) if root == self.volume.manifest_root() => Ok(()),
            Some(_) => Err(OwnershipError::Conflict),
            None => Err(OwnershipError::Unavailable),
        }
    }

    async fn load(&mut self, initialize: bool) -> Result<(), OwnershipError> {
        self.current_root().await?;
        self.volume
            .reconcile_pending_commit(self.authority.as_ref(), self.registry.as_ref())
            .await
            .map_err(|_| OwnershipError::Unavailable)?;
        self.current_root().await?;
        if initialize {
            if self.volume.manifest_root() != ManifestRoot::default()
                || !self.volume.manifest().list_entries("").is_empty()
            {
                return Err(OwnershipError::Configuration);
            }
            let initial = Record {
                version: 1,
                instance: self.instance,
                stream: self.stream,
                epoch: 0,
                writer_id: String::new(),
                claim_id: String::new(),
            };
            let bytes = serde_json::to_vec(&initial).map_err(|_| OwnershipError::InvalidRecord)?;
            self.volume
                .put(&self.path, &bytes)
                .await
                .map_err(|_| OwnershipError::Unavailable)?;
            self.volume
                .commit(self.authority.as_ref(), self.registry.as_ref())
                .await
                .map_err(|_| OwnershipError::Unavailable)?;
            self.current_root().await?;
        }
        // Even a zero root can belong to a USED volume after its last file is
        // removed. Missing control metadata is never a fresh epoch at reopen.
        let descriptor = self
            .volume
            .manifest()
            .get(&self.path)
            .ok_or(OwnershipError::InvalidRecord)?;
        if descriptor.plaintext_size(Visibility::Private) > MAX_RECORD_BYTES as u64 {
            return Err(OwnershipError::InvalidRecord);
        }
        let bytes = self
            .volume
            .get(&self.path)
            .await
            .map_err(|_| OwnershipError::Unavailable)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(OwnershipError::InvalidRecord);
        }
        let record: Record =
            serde_json::from_slice(&bytes).map_err(|_| OwnershipError::InvalidRecord)?;
        if record.version != 1
            || record.instance != self.instance
            || record.stream != self.stream
            || record.epoch == u64::MAX
            || (record.epoch == 0 && (!record.writer_id.is_empty() || !record.claim_id.is_empty()))
            || (record.epoch != 0
                && (!super::valid_identity(&record.writer_id)
                    || !super::valid_identity(&record.claim_id)))
        {
            return Err(OwnershipError::InvalidRecord);
        }
        self.record = Some(record);
        Ok(())
    }

    pub fn epoch(&self) -> Result<u64, OwnershipError> {
        if !self.usable {
            return Err(OwnershipError::Unavailable);
        }
        Ok(self.record.as_ref().map_or(0, |r| r.epoch))
    }

    /// CAS from the expected epoch. Exact claim retries return the same epoch;
    /// changed identity, lost CAS, unknown outcome or cancellation require a
    /// fresh authoritative open. Never increment/retry automatically on failure.
    /// The runtime must bind `writer_id` to one worker incarnation and its grant
    /// holder key, not a reusable hostname. A replacement process promotes with
    /// a new identity; it must not adopt another incarnation's successful claim.
    pub async fn claim(
        &mut self,
        expected_epoch: u64,
        writer_id: &str,
        claim_id: &str,
    ) -> Result<AllocatedWriter, OwnershipError> {
        if !self.usable {
            return Err(OwnershipError::Unavailable);
        }
        self.usable = false;
        let result = tokio::time::timeout(
            self.timeout,
            self.claim_inner(expected_epoch, writer_id, claim_id),
        )
        .await
        .map_err(|_| OwnershipError::Unavailable)
        .and_then(|r| r);
        if result.is_ok() {
            self.usable = true;
        }
        result
    }

    fn receipt(&self) -> AllocatedWriter {
        AllocatedWriter {
            record: self
                .record
                .as_ref()
                .expect("successful claim has a record")
                .clone(),
            control_volume: *self.volume.volume_id(),
            root: self.volume.manifest_root(),
        }
    }

    async fn claim_inner(
        &mut self,
        expected_epoch: u64,
        writer_id: &str,
        claim_id: &str,
    ) -> Result<AllocatedWriter, OwnershipError> {
        if !super::valid_identity(writer_id) || !super::valid_identity(claim_id) {
            return Err(OwnershipError::InvalidRecord);
        }
        let next = expected_epoch
            .checked_add(1)
            .filter(|e| *e != u64::MAX)
            .ok_or(OwnershipError::EpochExhausted)?;
        self.current_root().await?;
        if let Some(current) = &self.record {
            if current.claim_id == claim_id {
                return if current.writer_id == writer_id && current.epoch == next {
                    Ok(self.receipt())
                } else {
                    Err(OwnershipError::Conflict)
                };
            }
        }
        if self.record.as_ref().map_or(0, |r| r.epoch) != expected_epoch {
            return Err(OwnershipError::Conflict);
        }
        let record = Record {
            version: 1,
            instance: self.instance,
            stream: self.stream,
            epoch: next,
            writer_id: writer_id.into(),
            claim_id: claim_id.into(),
        };
        let bytes = serde_json::to_vec(&record).map_err(|_| OwnershipError::InvalidRecord)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(OwnershipError::InvalidRecord);
        }
        self.volume
            .put(&self.path, &bytes)
            .await
            .map_err(|_| OwnershipError::Unavailable)?;
        // An overlapping owner-record change must lose CBFS's prev-root CAS;
        // the SDK refuses overlapping rebase paths instead of overwriting them.
        self.volume
            .commit(self.authority.as_ref(), self.registry.as_ref())
            .await
            .map_err(|_| OwnershipError::Unavailable)?;
        self.load(false).await?;
        self.current_root().await?;
        if self.record.as_ref() != Some(&record) {
            return Err(OwnershipError::Conflict);
        }
        Ok(self.receipt())
    }
}
