//! CBQS owner-stream transport. This layer returns verified log records; it
//! neither allocates ownership nor acknowledges Cowchat commands to clients.
//! The caller must hold the unique stream epoch and provide archive durability.
mod archive;
#[cfg(feature = "cbfs-archive")]
pub(crate) use archive::unverified_archive_range;

use super::Command;
use cbqs_client::{CheckpointTrustV2, HeldRecord, SessionConfig, SessionV2, Socket};
use commonware_codec::Encode;
use cowboy_protocol_codec::cbqs_v2::{
    self as wire, CbqsRequestBodyV2 as Request, CbqsResponseBodyV2 as Response,
    CbqsServerFrameV2 as Frame,
};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("owner-stream configuration requires a finite epoch, full stream scope and a chain provider anchor")]
    Configuration,
    #[error("a newer owner epoch has already fenced this writer")]
    Fenced,
    #[error("CBQS operation failed; reacquire ownership and replay before retrying")]
    Unavailable,
    #[error("CBQS log no longer contains the required history; archive recovery is required")]
    HistoryGap,
    #[error("CBQS replay exceeds configured bounds; restore a verified archive checkpoint")]
    ReplayLimit,
    #[error("CBQS record or checkpoint verification failed")]
    Verification,
    #[error("owner-stream command encoding is invalid")]
    Encoding,
}

#[derive(Clone, Debug)]
pub struct LogRecord {
    pub sequence: u64,
    pub lane_id: u64,
    pub command: Command,
}

/// A boundary produced only after this module verified both the checkpoint
/// chain and its records. Deliberately not deserializable: archived records
/// must pass `restore_archive` verification to recreate this boundary.
#[derive(Clone, Debug)]
pub struct VerifiedCheckpoint {
    receipt: wire::CheckpointReceiptV2,
}

impl VerifiedCheckpoint {
    pub fn sequence(&self) -> u64 {
        self.receipt.last_sequence
    }

    pub fn id(&self) -> [u8; 32] {
        wire::checkpoint_id_v2(&self.receipt)
    }
}

#[derive(Debug)]
pub struct Replay {
    pub records: Vec<LogRecord>,
    pub checkpoint: Option<VerifiedCheckpoint>,
    archive: Option<archive::ArchiveData>,
}

pub struct CbqsOwnerLog {
    session: SessionV2,
    instance: [u8; 32],
    stream: [u8; 32],
    epoch: u64,
    holder: [u8; 32],
    usable: Arc<AtomicBool>,
    timeout: Duration,
}

// A caller may cancel a future, bypassing its normal error return. An
// interrupted request/replay must still retire the session before reuse.
struct RetireOnDrop {
    usable: Arc<AtomicBool>,
    completed: bool,
}
impl RetireOnDrop {
    fn new(usable: &Arc<AtomicBool>) -> Self {
        Self {
            usable: Arc::clone(usable),
            completed: false,
        }
    }
}
impl Drop for RetireOnDrop {
    fn drop(&mut self) {
        if !self.completed {
            self.usable.store(false, Ordering::Relaxed);
        }
    }
}

impl CbqsOwnerLog {
    #[cfg(feature = "cbfs-archive")]
    pub(crate) fn archive_identity(&self) -> ([u8; 32], [u8; 32]) {
        (self.instance, self.stream)
    }

    /// Socket routing belongs to the caller (checked registry resolution or
    /// trusted operator pinning). The provider anchor must come from chain
    /// authority, never the socket peer. A successful fence is not an ownership
    /// allocator: two callers must never receive the same writable epoch.
    pub async fn attach(
        fence_socket: Socket,
        session_socket: Socket,
        config: SessionConfig,
        sign_fence: impl FnOnce(&[u8]) -> [u8; 64],
        now_ms: u64,
    ) -> Result<Self, LogError> {
        let CheckpointTrustV2::ChainProvider(provider) = &config.checkpoints else {
            return Err(LogError::Configuration);
        };
        let epoch = config.grant.policy_epoch;
        let holder = config.holder.verifying_key().to_bytes();
        if config.grant.holder_signing_key.key_bytes != holder {
            return Err(LogError::Configuration);
        }
        // REPLAY admits historic cursors; their progress/credit/close operations
        // and signed checkpoint reads additionally require CONSUME.
        let required = wire::CBQS_V2_VERB_APPEND
            | wire::CBQS_V2_VERB_REPLAY
            | wire::CBQS_V2_VERB_CONSUME
            | wire::CBQS_V2_VERB_LANE_ADMIN;
        if epoch == 0
            || epoch == u64::MAX
            || config.grant.lane_scope != wire::LaneScopeV2::Any
            || config.grant.verbs & required != required
            || config.handshake_timeout.is_zero()
            || provider.chain_instance_id != config.grant.chain_instance_id
        {
            return Err(LogError::Configuration);
        }
        let instance = config.grant.chain_instance_id;
        let stream = config.grant.stream_id;
        let timeout = config.handshake_timeout;
        let fenced = tokio::time::timeout(
            timeout,
            cbqs_client::fence_policy_epoch(
                fence_socket,
                instance,
                stream,
                epoch,
                &provider.signing_key,
                sign_fence,
                now_ms,
            ),
        )
        .await
        .map_err(|_| LogError::Unavailable)?
        .map_err(|_| LogError::Unavailable)?;
        // The SDK deliberately accepts an idempotent request below the floor.
        // That is not permission for this owner to adopt the newer epoch.
        if fenced.ack.policy_epoch != epoch {
            return Err(LogError::Fenced);
        }
        let session = SessionV2::attach(session_socket, config, now_ms)
            .await
            .map_err(|_| LogError::Unavailable)?;
        Ok(Self {
            session,
            instance,
            stream,
            epoch,
            holder,
            usable: Arc::new(AtomicBool::new(true)),
            timeout,
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn holder_key(&self) -> [u8; 32] {
        self.holder
    }

    /// Keyed allocation is idempotent even if its response is lost. Room
    /// creation itself is a separate command on lane 0; an orphaned allocated
    /// lane is not a created room and grants no room access.
    pub async fn room_lane(&mut self, owner_id: &str, room_id: &str) -> Result<u64, LogError> {
        let encoded = serde_json::to_vec(&("cowchat/room-lane/1", owner_id, room_id))
            .map_err(|_| LogError::Encoding)?;
        let allocation_key: [u8; 32] = Sha256::digest(&encoded).into();
        match self
            .request(Request::CreateLaneForKey {
                allocation_key,
                metadata_hash: allocation_key,
            })
            .await?
        {
            Response::LaneCreated { lane_id } if lane_id != 0 => Ok(lane_id),
            _ => self.fail(LogError::Verification),
        }
    }

    /// The returned sequence is CBQS commitment, not Cowchat client success.
    /// The caller must apply the record and satisfy the archive gate first.
    /// On uncertainty, this session is retired; never retry an append blindly.
    pub async fn append(&mut self, lane_id: u64, command: &Command) -> Result<u64, LogError> {
        let payload = serde_json::to_vec(command).map_err(|_| LogError::Encoding)?;
        match self
            .request(Request::Append(wire::AppendRequestV2 { lane_id, payload }))
            .await?
        {
            Response::Appended { sequence } if sequence > 0 => Ok(sequence),
            _ => self.fail(LogError::Verification),
        }
    }

    /// Bounded initial recovery from genesis. Retention gaps refuse recovery;
    /// they never become an empty/new room. Use `replay` with a verified
    /// checkpoint when the caller already holds the prefix.
    /// `minimum_sequence` comes from durable ownership/archive recovery state;
    /// use zero only when no prior committed checkpoint is known.
    pub async fn replay_from_start(
        &mut self,
        minimum_sequence: u64,
        max_records: u64,
        max_bytes: usize,
    ) -> Result<Vec<LogRecord>, LogError> {
        Ok(self
            .replay(None, minimum_sequence, max_records, max_bytes)
            .await?
            .records)
    }

    /// Read a complete bounded suffix after a previously verified boundary.
    /// The caller must retain the corresponding prefix/projection. This is not
    /// archive restoration: arbitrary serialized checkpoints cannot enter here.
    /// Bounds apply to the suffix, independent of the stream's total age.
    pub async fn replay(
        &mut self,
        after: Option<&VerifiedCheckpoint>,
        minimum_sequence: u64,
        max_records: u64,
        max_bytes: usize,
    ) -> Result<Replay, LogError> {
        if !self.usable.load(Ordering::Relaxed) {
            return Err(LogError::Unavailable);
        }
        let mut operation = RetireOnDrop::new(&self.usable);
        let result = tokio::time::timeout(
            self.timeout,
            self.replay_inner(after, minimum_sequence, max_records, max_bytes),
        )
        .await
        .map_err(|_| LogError::Unavailable)
        .and_then(|result| result);
        operation.completed = result.is_ok();
        result
    }

    async fn replay_inner(
        &mut self,
        after: Option<&VerifiedCheckpoint>,
        minimum_sequence: u64,
        max_records: u64,
        max_bytes: usize,
    ) -> Result<Replay, LogError> {
        if after.is_some_and(|checkpoint| {
            checkpoint.receipt.chain_instance_id != self.instance
                || checkpoint.receipt.stream_id != self.stream
        }) {
            return Err(LogError::Verification);
        }
        let base = after.map_or(0, VerifiedCheckpoint::sequence);
        // Head binds the cursor to the durable stream tail, which survives
        // retention even when every checkpoint has expired. A missing latest
        // checkpoint by itself is never evidence of a new stream.
        let observed_tail = self.durable_tip().await?;
        if observed_tail < minimum_sequence.max(base) {
            return Err(LogError::HistoryGap);
        }
        if observed_tail == base {
            return Ok(Replay {
                records: Vec::new(),
                checkpoint: after.cloned(),
                archive: None,
            });
        }
        let first = base.checked_add(1).ok_or(LogError::ReplayLimit)?;
        let boundary = after.map_or_else(
            || wire::checkpoint_genesis_v2(&self.instance, &self.stream),
            |checkpoint| wire::checkpoint_id_v2(&checkpoint.receipt),
        );
        let mut chain = Vec::new();
        let mut fetch = None;
        loop {
            let response = tokio::time::timeout(
                self.timeout,
                self.session.request(Request::GetCheckpoint {
                    checkpoint_id: fetch,
                }),
            )
            .await
            .map_err(|_| LogError::Unavailable)?;
            let receipt = match response {
                Ok(Response::Checkpoint { receipt }) => receipt,
                // An absent latest checkpoint is ambiguous: retention can have
                // deleted every record. Do not silently treat it as genesis.
                Err(cbqs_client::TransportError::Broker(error))
                    if error.code == wire::CBQS_V2_ERR_CHECKPOINT_NOT_FOUND =>
                {
                    return Err(LogError::HistoryGap)
                }
                _ => return Err(LogError::Unavailable),
            };
            if receipt.first_sequence <= base {
                return Err(LogError::Verification);
            }
            if receipt.last_sequence.saturating_sub(base) > max_records
                || chain.len() as u64 >= max_records
            {
                return Err(LogError::ReplayLimit);
            }
            let previous = receipt.previous_checkpoint;
            chain.push(receipt);
            if previous == boundary {
                break;
            }
            fetch = Some(previous);
        }
        chain.reverse();
        let tail = chain.last().ok_or(LogError::Verification)?.last_sequence;
        if tail != observed_tail {
            return Err(LogError::Verification);
        }
        // CBQS cursors each read exactly one lane. Include default/control
        // lane 0 explicitly, enumerate the other lanes (including closed ones),
        // then merge their records by the signed stream sequence.
        let mut lanes = vec![0];
        let mut after_lane_id = None;
        loop {
            let Response::Lanes {
                lanes: page,
                next_after_lane_id,
            } = self
                .request(Request::ListLanes {
                    after_lane_id,
                    limit: 256,
                })
                .await?
            else {
                return Err(LogError::Verification);
            };
            if page.is_empty() {
                break;
            }
            for lane in &page {
                if lane.stream_id != self.stream || lane.lane_id <= *lanes.last().unwrap() {
                    return Err(LogError::Verification);
                }
                lanes.push(lane.lane_id);
            }
            if next_after_lane_id != page.last().map(|lane| lane.lane_id) {
                return Err(LogError::Verification);
            }
            after_lane_id = next_after_lane_id;
            // Lane count is independent of suffix record count: old rooms may
            // have no messages in this suffix. Bound enumeration separately.
            if lanes.len() > 16_385 {
                return Err(LogError::ReplayLimit);
            }
        }
        let mut all = std::collections::BTreeMap::new();
        let mut bytes = 0usize;
        for lane in lanes {
            self.read_lane(lane, first, tail, max_bytes, &mut bytes, &mut all)
                .await?;
        }
        if all.len() as u64 != tail - base || all.keys().copied().ne(first..=tail) {
            return Err(LogError::HistoryGap);
        }
        let delivered: Vec<_> = all
            .into_iter()
            .map(|(sequence, (header, payload))| (sequence, header, payload))
            .collect();
        self.verified_replay(after, chain, delivered, max_records, max_bytes)
    }

    fn verified_replay(
        &self,
        after: Option<&VerifiedCheckpoint>,
        chain: Vec<wire::CheckpointReceiptV2>,
        delivered: Vec<(u64, wire::RecordHeaderV2, Vec<u8>)>,
        max_records: u64,
        max_bytes: usize,
    ) -> Result<Replay, LogError> {
        if after.is_some_and(|checkpoint| {
            checkpoint.receipt.chain_instance_id != self.instance
                || checkpoint.receipt.stream_id != self.stream
        }) {
            return Err(LogError::Verification);
        }
        self.session
            .verify_chain(
                &chain,
                after.map_or(cbqs_client::Predecessor::Genesis, |checkpoint| {
                    cbqs_client::Predecessor::Receipt(&checkpoint.receipt)
                }),
            )
            .map_err(|_| LogError::Verification)?;
        let base = after.map_or(0, VerifiedCheckpoint::sequence);
        let tail = chain.last().ok_or(LogError::Verification)?.last_sequence;
        let count = tail.checked_sub(base).ok_or(LogError::Verification)?;
        if count > max_records || delivered.len() as u64 > max_records {
            return Err(LogError::ReplayLimit);
        }
        if count == 0 || delivered.len() as u64 != count {
            return Err(LogError::HistoryGap);
        }
        let mut bytes = 0usize;
        for (offset, (sequence, header, payload)) in delivered.iter().enumerate() {
            if base
                .checked_add(offset as u64)
                .and_then(|n| n.checked_add(1))
                != Some(*sequence)
                || header.stream_id != self.stream
            {
                return Err(LogError::Verification);
            }
            bytes = bytes
                .checked_add(payload.len())
                .ok_or(LogError::ReplayLimit)?;
            if bytes > max_bytes {
                return Err(LogError::ReplayLimit);
            }
        }
        let mut checked = 0usize;
        for receipt in &chain {
            let count = receipt
                .last_sequence
                .checked_sub(receipt.first_sequence)
                .and_then(|n| n.checked_add(1))
                .ok_or(LogError::Verification)?;
            let end = checked
                .checked_add(usize::try_from(count).map_err(|_| LogError::ReplayLimit)?)
                .ok_or(LogError::ReplayLimit)?;
            let batch = delivered.get(checked..end).ok_or(LogError::Verification)?;
            let held: Vec<_> = batch
                .iter()
                .map(|(_, header, payload)| HeldRecord {
                    header: header.clone(),
                    payload,
                })
                .collect();
            SessionV2::verify_records(receipt, &held).map_err(|_| LogError::Verification)?;
            checked = end;
        }
        if checked != delivered.len() {
            return Err(LogError::Verification);
        }
        let records = delivered
            .iter()
            .map(|(sequence, header, payload)| {
                Ok(LogRecord {
                    sequence: *sequence,
                    lane_id: header.lane_id,
                    command: serde_json::from_slice(payload).map_err(|_| LogError::Encoding)?,
                })
            })
            .collect::<Result<Vec<_>, LogError>>()?;
        Ok(Replay {
            records,
            checkpoint: chain
                .last()
                .cloned()
                .map(|receipt| VerifiedCheckpoint { receipt }),
            archive: Some(archive::ArchiveData {
                receipts: chain,
                records: delivered,
            }),
        })
    }

    async fn durable_tip(&mut self) -> Result<u64, LogError> {
        let mut subscription_id = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut subscription_id);
        self.request(Request::OpenSubscription {
            subscription_id,
            source: wire::SubscriptionSourceV2::Cursor {
                lane_scope: wire::LaneScopeV2::Exact(0),
                start: wire::StartV2::Head,
            },
        })
        .await?;
        let response = self
            .request(Request::GetCursorProgress { subscription_id })
            .await?;
        self.request(Request::CloseSubscription { subscription_id })
            .await?;
        match response {
            Response::CursorProgress {
                subscription_id: id,
                lane_id: 0,
                scanned_through,
            } if id == subscription_id => Ok(scanned_through),
            _ => Err(LogError::Verification),
        }
    }

    async fn read_lane(
        &mut self,
        lane_id: u64,
        first: u64,
        tail: u64,
        max_bytes: usize,
        bytes: &mut usize,
        all: &mut std::collections::BTreeMap<u64, (wire::RecordHeaderV2, Vec<u8>)>,
    ) -> Result<(), LogError> {
        let mut subscription_id = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut subscription_id);
        self.request(Request::OpenSubscription {
            subscription_id,
            source: wire::SubscriptionSourceV2::Cursor {
                lane_scope: wire::LaneScopeV2::Exact(lane_id),
                start: wire::StartV2::At(first),
            },
        })
        .await?;
        const CREDIT: u32 = 1024 * 1024;
        self.request(Request::Credit {
            subscription_id,
            bytes: CREDIT,
        })
        .await?;
        let mut previous = first - 1;
        loop {
            let Response::CursorProgress {
                subscription_id: progress_id,
                lane_id: progress_lane,
                scanned_through,
            } = self
                .request(Request::GetCursorProgress { subscription_id })
                .await?
            else {
                return Err(LogError::Verification);
            };
            if progress_id != subscription_id || progress_lane != lane_id {
                return Err(LogError::Verification);
            }
            // request() queues deliveries which precede its response. Drain
            // them BEFORE interpreting scanned_through as a replay boundary.
            let mut replenish = 0u32;
            while let Some(frame) = self.session.try_next_event() {
                let wire_bytes =
                    u32::try_from(frame.encode().len()).map_err(|_| LogError::ReplayLimit)?;
                match frame {
                    Frame::Delivery {
                        subscription_id: id,
                        sequence,
                        header,
                        lease,
                        payload,
                        ..
                    } if id == subscription_id
                        && sequence > previous
                        && sequence <= tail
                        && header.stream_id == self.stream
                        && header.lane_id == lane_id
                        && lease.is_none() =>
                    {
                        *bytes = bytes
                            .checked_add(payload.len())
                            .ok_or(LogError::ReplayLimit)?;
                        if *bytes > max_bytes {
                            return Err(LogError::ReplayLimit);
                        }
                        previous = sequence;
                        if all.insert(sequence, (header, payload)).is_some() {
                            return Err(LogError::Verification);
                        }
                        replenish = replenish
                            .checked_add(wire_bytes)
                            .ok_or(LogError::ReplayLimit)?;
                    }
                    // A state queued before the initial/replenishing Credit is
                    // normal. A frame larger than the window cannot make progress.
                    Frame::DeliveryState {
                        subscription_id: id,
                        state: wire::DeliveryStateBodyV2::CreditShort { needed_bytes },
                        ..
                    } if id == subscription_id && needed_bytes <= CREDIT => {}
                    _ => return Err(LogError::Verification),
                }
            }
            if scanned_through >= tail {
                break;
            }
            if replenish > 0 {
                self.request(Request::Credit {
                    subscription_id,
                    bytes: replenish,
                })
                .await?;
            } else {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        self.request(Request::CloseSubscription { subscription_id })
            .await?;
        Ok(())
    }

    async fn request(&mut self, body: Request) -> Result<Response, LogError> {
        if !self.usable.load(Ordering::Relaxed) {
            return Err(LogError::Unavailable);
        }
        let mut operation = RetireOnDrop::new(&self.usable);
        match tokio::time::timeout(self.timeout, self.session.request(body)).await {
            Ok(Ok(response)) => {
                operation.completed = true;
                Ok(response)
            }
            Ok(Err(cbqs_client::TransportError::Broker(error)))
                if error.code == wire::CBQS_V2_ERR_CURSOR_TOO_OLD
                    || error.code == wire::CBQS_V2_ERR_CHECKPOINT_NOT_FOUND =>
            {
                self.fail(LogError::HistoryGap)
            }
            Ok(Err(cbqs_client::TransportError::Broker(error)))
                if error.code == wire::CBQS_V2_ERR_POLICY_EPOCH_STALE =>
            {
                self.fail(LogError::Fenced)
            }
            _ => self.fail(LogError::Unavailable),
        }
    }
    fn fail<T>(&mut self, error: LogError) -> Result<T, LogError> {
        self.usable.store(false, Ordering::Relaxed);
        Err(error)
    }
}
