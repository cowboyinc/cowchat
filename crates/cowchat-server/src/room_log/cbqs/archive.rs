//! Storage-neutral encoding for one complete signed checkpoint range. CBFS
//! publication/discovery belongs to the archive writer; these bytes alone are
//! not evidence that a batch has reached durable storage.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use commonware_codec::DecodeExt;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub(super) struct ArchiveData {
    pub receipts: Vec<wire::CheckpointReceiptV2>,
    pub records: Vec<(u64, wire::RecordHeaderV2, Vec<u8>)>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    version: u8,
    receipts: Vec<String>,
    records: Vec<Record>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    sequence: u64,
    header: String,
    payload: String,
}

impl Replay {
    /// Preserve the exact signed wire bytes, not reserialized commands or a
    /// trusted-looking snapshot. Empty replay produces no new archive segment.
    /// Publishing these bytes and recording a durable head is a separate gate.
    pub fn archive_bytes(&self, max_bytes: usize) -> Result<Option<Vec<u8>>, LogError> {
        let Some(archive) = &self.archive else {
            return Ok(None);
        };
        let segment = Segment {
            version: 1,
            receipts: archive
                .receipts
                .iter()
                .map(|r| STANDARD.encode(r.encode()))
                .collect(),
            records: archive
                .records
                .iter()
                .map(|(sequence, header, payload)| Record {
                    sequence: *sequence,
                    header: STANDARD.encode(header.encode()),
                    payload: STANDARD.encode(payload),
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&segment).map_err(|_| LogError::Encoding)?;
        if bytes.len() > max_bytes {
            return Err(LogError::ReplayLimit);
        }
        Ok(Some(bytes))
    }
}

impl CbqsOwnerLog {
    /// Verify bytes retrieved from an archive against this session's provider
    /// authority and the previously verified prefix. No broker history is read.
    /// A forged, reordered, truncated or foreign segment cannot create a
    /// VerifiedCheckpoint. The caller must separately authenticate the archive
    /// head/minimum sequence to detect rollback of an otherwise valid prefix.
    pub fn restore_archive(
        &mut self,
        bytes: &[u8],
        after: Option<&VerifiedCheckpoint>,
        max_records: u64,
        max_bytes: usize,
    ) -> Result<Replay, LogError> {
        if !self.usable.load(Ordering::Relaxed) {
            return Err(LogError::Unavailable);
        }
        let result = self.restore_archive_inner(bytes, after, max_records, max_bytes);
        if result.is_err() {
            self.usable.store(false, Ordering::Relaxed);
        }
        result
    }

    fn restore_archive_inner(
        &self,
        bytes: &[u8],
        after: Option<&VerifiedCheckpoint>,
        max_records: u64,
        max_bytes: usize,
    ) -> Result<Replay, LogError> {
        // Check encoded size before JSON/base64 allocations. Decoded payloads
        // are bounded again in the shared network/archive verifier.
        if bytes.len() > max_bytes {
            return Err(LogError::ReplayLimit);
        }
        let segment: Segment = serde_json::from_slice(bytes).map_err(|_| LogError::Encoding)?;
        if segment.version != 1 {
            return Err(LogError::Encoding);
        }
        if segment.records.len() as u64 > max_records || segment.receipts.len() as u64 > max_records
        {
            return Err(LogError::ReplayLimit);
        }
        let receipts = segment
            .receipts
            .iter()
            .map(|encoded| {
                let bytes = STANDARD.decode(encoded).map_err(|_| LogError::Encoding)?;
                wire::CheckpointReceiptV2::decode(bytes.as_slice()).map_err(|_| LogError::Encoding)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let records = segment
            .records
            .into_iter()
            .map(|r| {
                let header = STANDARD.decode(r.header).map_err(|_| LogError::Encoding)?;
                let header = wire::RecordHeaderV2::decode(header.as_slice())
                    .map_err(|_| LogError::Encoding)?;
                let payload = STANDARD.decode(r.payload).map_err(|_| LogError::Encoding)?;
                Ok((r.sequence, header, payload))
            })
            .collect::<Result<Vec<_>, LogError>>()?;
        self.verified_replay(after, receipts, records, max_records, max_bytes)
    }
}
