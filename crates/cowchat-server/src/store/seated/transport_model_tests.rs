//! Executable design model, NOT a production adapter or durability proof.
//! `journal`, `broker` and `archive` are simulated durable stores; crash drops
//! only staged admission. Envelope verification is real. Evidence uses a
//! separate, explicitly test-only signed format, NOT CBQS/CBFS receipt codecs.
//! Stable test/scenario names are intended for the future real adapter suite.
use super::*;
use ed25519_dalek::{Signer, SigningKey};

type ModelResult<T> = Result<T, &'static str>;

#[derive(Clone)]
struct Intent {
    id: String,
    digest: Vec<u8>,
    bytes: Vec<u8>,
    epoch: u64,
    auth: u64,
    transport: Option<u64>,
    position: Option<u64>,
    archived: bool,
    canceled: bool,
    wake_count: u64,
}

#[derive(Clone)]
struct Evidence {
    archive: bool,
    digest: Vec<u8>,
    sequence: u64,
    epoch: u64,
    auth: u64,
    signature: Vec<u8>,
}

impl Evidence {
    fn preimage(&self) -> Vec<u8> {
        encode(&(
            "cowchat-test-model-evidence-only",
            self.archive,
            &self.digest,
            self.sequence,
            self.epoch,
            self.auth,
        ))
    }
    fn fixture_key(archive: bool) -> SigningKey {
        SigningKey::from_bytes(&[if archive { 92 } else { 91 }; 32])
    }
    fn issue(intent: &Intent, sequence: u64, archive: bool) -> Self {
        let mut result = Self {
            archive,
            digest: intent.digest.clone(),
            sequence,
            epoch: intent.epoch,
            auth: intent.auth,
            signature: vec![],
        };
        result.signature = Self::fixture_key(archive)
            .sign(&result.preimage())
            .to_bytes()
            .to_vec();
        result
    }
    fn verify(&self) -> ModelResult<()> {
        let signature =
            ed25519_dalek::Signature::from_slice(&self.signature).map_err(|_| "forged")?;
        Self::fixture_key(self.archive)
            .verifying_key()
            .verify_strict(&self.preimage(), &signature)
            .map_err(|_| "forged")
    }
}

#[derive(Clone)]
struct Model {
    context: Vec<u8>,
    now: u64,
    epoch: u64,
    auth: u64,
    journal: Vec<Intent>,
    staged: Option<Intent>,
    broker: Vec<(Vec<u8>, Evidence)>,
    archive: Vec<Evidence>,
    sent: Vec<String>,
}

impl Model {
    fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        install_fixture(&store);
        let context = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT trusted_context FROM seated_credentials WHERE cert_id='fixture-cert'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        Self {
            context,
            now: Utc::now().timestamp_millis() as u64,
            epoch: 1,
            auth: 1,
            journal: vec![],
            staged: None,
            broker: vec![],
            archive: vec![],
            sent: vec![],
        }
    }

    fn begin(&mut self, bytes: &[u8]) -> ModelResult<usize> {
        // The fixture context was issued at auth revision 1. This model has no
        // authority re-enrollment path; a revision change invalidates it.
        if self.auth != 1 {
            return Err("fenced");
        }
        let record = crate::seated::SealedRecord::parse(bytes).map_err(|_| "invalid envelope")?;
        authorization::verify_member_record(
            &record.header_cbor,
            &record.body,
            &record.signature,
            &self.context,
            self.now,
        )
        .map_err(|_| "invalid envelope")?;
        let digest = Sha256::digest(encode(&(
            &record.header_cbor,
            &record.body,
            &record.signature,
        )))
        .to_vec();
        if let Some((index, intent)) = self
            .journal
            .iter()
            .enumerate()
            .find(|(_, i)| i.id == record.header.message_id)
        {
            return if intent.digest == digest {
                Ok(index)
            } else {
                Err("identity conflict")
            };
        }
        if self.staged.is_some() {
            return Err("uncommitted admission");
        }
        self.staged = Some(Intent {
            id: record.header.message_id,
            digest,
            bytes: bytes.to_vec(),
            epoch: self.epoch,
            auth: self.auth,
            transport: None,
            position: None,
            archived: false,
            canceled: false,
            wake_count: 0,
        });
        Ok(self.journal.len())
    }

    fn commit_intent(&mut self) {
        if let Some(intent) = self.staged.take() {
            self.journal.push(intent);
        }
    }
    fn eligible(&self, index: usize) -> ModelResult<()> {
        let intent = self.journal.get(index).ok_or("no durable intent")?;
        if intent.canceled || intent.epoch != self.epoch || intent.auth != self.auth {
            return Err("fenced");
        }
        Ok(())
    }
    fn broker_append(&mut self, index: usize) -> ModelResult<Evidence> {
        self.eligible(index)?;
        let evidence = Evidence::issue(&self.journal[index], self.broker.len() as u64 + 1, false);
        self.broker
            .push((self.journal[index].bytes.clone(), evidence.clone()));
        Ok(evidence)
    }
    fn accept(&mut self, index: usize, evidence: &Evidence, archive: bool) -> ModelResult<()> {
        // Already-published history still needs archiving after a later fence.
        // Its immutable admission revision is evidence binding, not permission
        // to publish a new message under an obsolete writer.
        if !archive || self.journal[index].position.is_none() {
            self.eligible(index)?;
        }
        evidence.verify()?;
        let intent = &mut self.journal[index];
        if evidence.archive != archive
            || evidence.digest != intent.digest
            || evidence.sequence == 0
            || evidence.epoch != intent.epoch
            || evidence.auth != intent.auth
        {
            return Err("mismatched evidence");
        }
        if archive {
            intent.archived = true;
        } else {
            intent.transport = Some(evidence.sequence);
        }
        Ok(())
    }
    fn publish(&mut self, index: usize) -> ModelResult<()> {
        if self.journal[index].position.is_some() {
            return Ok(());
        }
        self.eligible(index)?;
        if self.journal[index].transport.is_none() {
            return Err("transport pending");
        }
        if self.journal[..index]
            .iter()
            .any(|i| i.position.is_none() && !i.canceled)
        {
            return Err("earlier admission pending");
        }
        let position = self.journal.iter().filter(|i| i.position.is_some()).count() as u64 + 1;
        self.journal[index].position = Some(position);
        self.journal[index].wake_count = 1;
        Ok(())
    }
    fn archive_commit(&mut self, index: usize) -> ModelResult<Evidence> {
        if self.journal[index].position.is_none() {
            self.eligible(index)?;
        }
        let evidence = Evidence::issue(&self.journal[index], self.archive.len() as u64 + 1, true);
        self.archive.push(evidence.clone());
        Ok(evidence)
    }
    fn acknowledge_sent(&mut self, index: usize) -> ModelResult<()> {
        let intent = &self.journal[index];
        if intent.position.is_none() || !intent.archived {
            return Err("sent pending");
        }
        if !self.sent.contains(&intent.id) {
            self.sent.push(intent.id.clone());
        }
        Ok(())
    }
    fn fence(&mut self, writer: bool) {
        if writer {
            self.epoch += 1;
        } else {
            self.auth += 1;
        }
        for intent in &mut self.journal {
            if intent.position.is_none() {
                intent.canceled = true;
            }
        }
    }
    fn crash(&mut self) {
        self.staged = None;
    }
    fn restore(snapshot: Self, epoch_floor: u64, auth_floor: u64) -> ModelResult<Self> {
        if snapshot.epoch < epoch_floor || snapshot.auth < auth_floor {
            return Err("stale restore");
        }
        Ok(snapshot)
    }
}

fn wire(index: u8) -> Vec<u8> {
    let mut header = crate::seated::SealedRecord::parse(&sealed(b"model template"))
        .unwrap()
        .header;
    header.message_id =
        uuid::Uuid::from_u128(0x20000000000040008000000000000000 + u128::from(index)).to_string();
    header.nonce = B64.encode([index + 1; 12]);
    seal_header(header, b"private model fixture")
}

#[test]
fn archive_stall_publishes_and_wakes_but_final_sent_waits() {
    let mut model = Model::new();
    for index in 0..2 {
        model.begin(&wire(index as u8)).unwrap();
        model.commit_intent();
        assert_eq!(model.publish(index), Err("transport pending"));
        let receipt = model.broker_append(index).unwrap();
        model.accept(index, &receipt, false).unwrap();
        model.publish(index).unwrap();
        assert_eq!(model.acknowledge_sent(index), Err("sent pending"));
        assert_eq!(model.journal[index].position, Some(index as u64 + 1));
        assert_eq!(model.journal[index].wake_count, 1);
    }
    assert!(model.archive.is_empty());
    assert!(model.sent.is_empty());
    let receipt = model.archive_commit(0).unwrap();
    model.accept(0, &receipt, true).unwrap();
    model.acknowledge_sent(0).unwrap();
    model.acknowledge_sent(0).unwrap();
    assert_eq!(model.sent.len(), 1);
    assert_eq!(model.journal[0].wake_count, 1);
}

#[test]
fn crash_at_each_boundary_recovers_one_logical_record_and_one_wake() {
    // A stable schedule name accompanies every assertion for adapter reuse.
    let names = [
        "before_intent_commit",
        "after_intent_commit",
        "broker_accept_reply_lost",
        "after_transport_evidence",
        "after_publish",
        "archive_commit_reply_lost",
        "after_archive_evidence",
        "sent_ack_reply_lost",
    ];
    for (cut, name) in names.iter().enumerate() {
        let mut model = Model::new();
        let bytes = wire(0);
        model.begin(&bytes).unwrap();
        assert_eq!(
            model.broker_append(0).err(),
            Some("no durable intent"),
            "{name}"
        );
        if cut >= 1 {
            model.commit_intent();
        }
        if cut >= 2 {
            let r = model.broker_append(0).unwrap();
            if cut >= 3 {
                model.accept(0, &r, false).unwrap();
            }
        }
        if cut >= 4 {
            model.publish(0).unwrap();
        }
        if cut >= 5 {
            let r = model.archive_commit(0).unwrap();
            if cut >= 6 {
                model.accept(0, &r, true).unwrap();
            }
        }
        if cut >= 7 {
            model.acknowledge_sent(0).unwrap();
        }
        model.crash();
        model.begin(&bytes).unwrap();
        model.commit_intent();
        if model.journal[0].transport.is_none() {
            // No automatic broker dedupe: ambiguous acceptance creates another
            // physical record, but never another intent/logical room position.
            let r = model.broker_append(0).unwrap();
            model.accept(0, &r, false).unwrap();
        }
        model.publish(0).unwrap();
        if !model.journal[0].archived {
            let r = model
                .archive
                .first()
                .cloned()
                .unwrap_or_else(|| model.archive_commit(0).unwrap());
            model.accept(0, &r, true).unwrap();
        }
        model.acknowledge_sent(0).unwrap();
        assert_eq!(model.journal.len(), 1, "{name}");
        assert_eq!(model.journal[0].position, Some(1), "{name}");
        assert_eq!(model.journal[0].wake_count, 1, "{name}");
        assert_eq!(model.sent.len(), 1, "{name}");
        assert_eq!(model.broker.len(), if cut == 2 { 2 } else { 1 }, "{name}");
        assert!(
            model.broker.iter().all(|(wire, _)| wire == &bytes),
            "{name}"
        );
    }
}

#[test]
fn writer_or_auth_fence_before_publish_rejects_inflight_and_stale_restore() {
    for writer in [false, true] {
        for evidence_before_fence in [false, true] {
            let mut model = Model::new();
            model.begin(&wire(0)).unwrap();
            model.commit_intent();
            let receipt = model.broker_append(0).unwrap();
            if evidence_before_fence {
                model.accept(0, &receipt, false).unwrap();
            }
            let stale = model.clone();
            model.fence(writer);
            assert_eq!(model.accept(0, &receipt, false), Err("fenced"));
            assert_eq!(model.publish(0), Err("fenced"));
            assert_eq!(model.broker_append(0).err(), Some("fenced"));
            assert_eq!(model.journal[0].position, None);
            assert_eq!(model.journal[0].wake_count, 0);
            if !writer {
                assert_eq!(model.begin(&wire(1)), Err("fenced"));
            }
            assert_eq!(model.broker.len(), 1); // In-flight encrypted bytes can remain.
            assert_eq!(
                Model::restore(stale, model.epoch, model.auth)
                    .err()
                    .map(|e| e.to_owned()),
                Some("stale restore".into())
            );
        }
    }
}

#[test]
fn fence_after_publish_preserves_visibility_and_allows_archive_completion_only() {
    for writer in [false, true] {
        let mut model = Model::new();
        model.begin(&wire(0)).unwrap();
        model.commit_intent();
        let receipt = model.broker_append(0).unwrap();
        model.accept(0, &receipt, false).unwrap();
        model.publish(0).unwrap();
        model.fence(writer);
        assert_eq!(model.accept(0, &receipt, false), Err("fenced"));
        assert_eq!(model.broker_append(0).err(), Some("fenced"));
        let archive = model.archive_commit(0).unwrap();
        model.accept(0, &archive, true).unwrap();
        model.acknowledge_sent(0).unwrap();
        assert_eq!(model.journal[0].position, Some(1));
        assert_eq!(model.journal[0].wake_count, 1);
        assert_eq!(model.sent.len(), 1);
    }
}

#[test]
fn forged_or_mismatched_evidence_and_changed_identity_fail_closed() {
    let mut model = Model::new();
    let original = wire(0);
    model.begin(&original).unwrap();
    model.commit_intent();
    let receipt = model.broker_append(0).unwrap();
    let mut forged = receipt.clone();
    forged.sequence += 1;
    assert_eq!(model.accept(0, &forged, false), Err("forged"));
    for mutation in ["digest", "epoch", "auth", "kind", "zero_sequence"] {
        let mut wrong = receipt.clone();
        match mutation {
            "digest" => wrong.digest[0] ^= 1,
            "epoch" => wrong.epoch += 1,
            "auth" => wrong.auth += 1,
            "kind" => wrong.archive = true,
            _ => wrong.sequence = 0,
        }
        wrong.signature = Evidence::fixture_key(wrong.archive)
            .sign(&wrong.preimage())
            .to_bytes()
            .to_vec();
        assert_eq!(
            model.accept(0, &wrong, false),
            Err("mismatched evidence"),
            "{mutation}"
        );
    }
    assert!(model.journal[0].transport.is_none());
    assert_eq!(model.publish(0), Err("transport pending"));
    let mut header = crate::seated::SealedRecord::parse(&original)
        .unwrap()
        .header;
    header.nonce = B64.encode([77; 12]);
    let changed = seal_header(header, b"different authenticated candidate");
    assert_eq!(model.begin(&changed), Err("identity conflict"));
    let mut forged_wire: serde_json::Value = serde_json::from_slice(&original).unwrap();
    forged_wire["seat"] = "0x2222222222222222222222222222222222222222".into();
    assert_eq!(
        model.begin(&serde_json::to_vec(&forged_wire).unwrap()),
        Err("invalid envelope")
    );
    assert_eq!(model.journal.len(), 1);
    assert!(model.staged.is_none());
}
