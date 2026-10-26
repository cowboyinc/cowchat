//! Durable, room-scoped coordination. Transport presence never owns a lease.
use super::Store;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const HEARTBEAT_SECONDS: i64 = 60;
pub const OVERDUE_SECONDS: i64 = 300;
pub const EXPIRY_SECONDS: i64 = 420;
// Keep worst-case JSON listings below the 1 MiB transport frame limit.
const MAX_RESOURCES: i64 = 128;

pub(super) fn initialize(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS room_leases (
        room TEXT NOT NULL REFERENCES rooms(room_id), name TEXT NOT NULL,
        generation INTEGER NOT NULL, principal TEXT NOT NULL, holder TEXT NOT NULL,
        holder_name TEXT NOT NULL, state TEXT NOT NULL,
        detail TEXT, progress TEXT, acquired INTEGER NOT NULL, heartbeat INTEGER NOT NULL,
        progressed INTEGER NOT NULL, released INTEGER, PRIMARY KEY(room,name));
        CREATE TABLE IF NOT EXISTS room_readiness (
        room TEXT NOT NULL REFERENCES rooms(room_id), name TEXT NOT NULL, principal TEXT NOT NULL,
        reviewer TEXT NOT NULL, reviewer_name TEXT NOT NULL, commit_sha TEXT NOT NULL,
        verdict TEXT NOT NULL, reason TEXT NOT NULL, updated INTEGER NOT NULL,
        PRIMARY KEY(room,name,principal,reviewer));",
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub room_id: String,
    pub operation: Operation,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    List {},
    Get {
        name: String,
    },
    Acquire {
        name: String,
        expected_generation: i64,
    },
    Heartbeat {
        name: String,
        generation: i64,
        state: Option<TaskState>,
        detail: Option<String>,
        progress: Option<String>,
    },
    Release {
        name: String,
        generation: i64,
    },
    Validate {
        name: String,
        generation: i64,
        holder: String,
    },
    Readiness {
        name: String,
        commit: String,
        verdict: Verdict,
        reason: String,
    },
    ReadinessList {
        name: String,
        commit: String,
    },
}
impl Operation {
    pub fn mutates(&self) -> bool {
        !matches!(
            self,
            Self::List {} | Self::Get { .. } | Self::Validate { .. } | Self::ReadinessList { .. }
        )
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Working,
    WaitingForHuman,
    WaitingForDependency,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ready,
    Blocked,
    Withdrawn,
}
#[derive(Debug, thiserror::Error)]
pub enum CoordinationError {
    #[error("Invalid coordination request: {0}")]
    Invalid(&'static str),
    #[error("Resource already leased; inspect its holder before retrying")]
    Conflict,
    #[error("Lease lost, expired, released, or not owned by this agent")]
    Lost,
    #[error("Coordination storage unavailable")]
    Storage(#[from] rusqlite::Error),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Lease {
    pub room_id: String,
    pub name: String,
    pub generation: i64,
    pub holder_id: String,
    pub holder_name: String,
    pub task_state: TaskState,
    pub detail: Option<String>,
    pub progress: Option<String>,
    pub acquired_at: i64,
    pub last_heartbeat_at: i64,
    pub last_progress_at: i64,
    pub overdue_at: i64,
    pub expires_at: i64,
    pub released_at: Option<i64>,
    pub status: String,
}
struct Record {
    lease: Lease,
    principal: String,
}
const COLUMNS: &str = "room,name,generation,holder,holder_name,state,detail,progress,acquired,heartbeat,progressed,released,principal";
fn row(row: &rusqlite::Row<'_>, now: i64) -> rusqlite::Result<Record> {
    let state: String = row.get(5)?;
    let heartbeat: i64 = row.get(9)?;
    let released: Option<i64> = row.get(11)?;
    Ok(Record {
        lease: Lease {
            room_id: row.get(0)?,
            name: row.get(1)?,
            generation: row.get(2)?,
            holder_id: row.get(3)?,
            holder_name: row.get(4)?,
            task_state: match state.as_str() {
                "waiting_for_human" => TaskState::WaitingForHuman,
                "waiting_for_dependency" => TaskState::WaitingForDependency,
                _ => TaskState::Working,
            },
            detail: row.get(6)?,
            progress: row.get(7)?,
            acquired_at: row.get(8)?,
            last_heartbeat_at: heartbeat,
            last_progress_at: row.get(10)?,
            overdue_at: heartbeat.saturating_add(OVERDUE_SECONDS),
            expires_at: heartbeat.saturating_add(EXPIRY_SECONDS),
            released_at: released,
            status: if released.is_some() {
                "released"
            } else if now >= heartbeat.saturating_add(EXPIRY_SECONDS) {
                "expired"
            } else if now >= heartbeat.saturating_add(OVERDUE_SECONDS) {
                "overdue"
            } else {
                "active"
            }
            .into(),
        },
        principal: row.get(12)?,
    })
}
fn bounded(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}
fn resource(name: &str) -> Result<(), CoordinationError> {
    if name.is_empty() || name.trim() != name || !bounded(name, 160) {
        return Err(CoordinationError::Invalid(
            "resource must contain 1–160 bytes without controls or outer whitespace",
        ));
    }
    Ok(())
}
fn commit(value: &str) -> Result<(), CoordinationError> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(CoordinationError::Invalid(
            "commit must be a full lowercase 40- or 64-digit SHA",
        ));
    }
    Ok(())
}
fn state_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Working => "working",
        TaskState::WaitingForHuman => "waiting_for_human",
        TaskState::WaitingForDependency => "waiting_for_dependency",
    }
}

impl Store {
    /// Authorization is performed by the registered-session handler before this transaction.
    pub fn coordinate(
        &self,
        room: &str,
        principal: &str,
        agent: &str,
        agent_name: &str,
        operation: Operation,
        now: i64,
    ) -> Result<Value, CoordinationError> {
        // Identity strings are echoed in listings; bound them as well as user details.
        if agent.is_empty() || !bounded(agent, 256) || !bounded(agent_name, 256) {
            return Err(CoordinationError::Invalid(
                "coordination identities must fit 256 bytes without controls",
            ));
        }
        let mut db = self.conn.lock().unwrap();
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = match operation {
            Operation::List {} => {
                let mut statement = tx.prepare(&format!(
                    "SELECT {COLUMNS} FROM room_leases WHERE room=?1 ORDER BY name"
                ))?;
                let leases = statement
                    .query_map([room], |r| row(r, now).map(|r| r.lease))?
                    .collect::<Result<Vec<_>, _>>()?;
                json!({"leases":leases,"heartbeat_seconds":HEARTBEAT_SECONDS,"overdue_seconds":OVERDUE_SECONDS,"expiry_seconds":EXPIRY_SECONDS})
            }
            Operation::Readiness {
                name,
                commit: sha,
                verdict,
                reason,
            } => {
                resource(&name)?;
                commit(&sha)?;
                if !bounded(&reason, 2048) {
                    return Err(CoordinationError::Invalid(
                        "readiness reason exceeds 2048 bytes or contains controls",
                    ));
                }
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM room_readiness WHERE room=?1",
                    [room],
                    |r| r.get(0),
                )?;
                let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM room_readiness WHERE room=?1 AND name=?2 AND principal=?3 AND reviewer=?4)",params![room,name,principal,agent],|r|r.get(0))?;
                if count >= MAX_RESOURCES && !exists {
                    return Err(CoordinationError::Invalid(
                        "room readiness record limit reached",
                    ));
                }
                let verdict = match verdict {
                    Verdict::Ready => "ready",
                    Verdict::Blocked => "blocked",
                    Verdict::Withdrawn => "withdrawn",
                };
                tx.execute("INSERT INTO room_readiness(room,name,principal,reviewer,reviewer_name,commit_sha,verdict,reason,updated) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
                    ON CONFLICT(room,name,principal,reviewer) DO UPDATE SET reviewer_name=excluded.reviewer_name,commit_sha=excluded.commit_sha,verdict=excluded.verdict,reason=excluded.reason,updated=excluded.updated",
                    params![room,name,principal,agent,agent_name,sha,verdict,reason,now])?;
                json!({"name":name,"commit":sha,"reviewer_id":agent,"verdict":verdict,"reason":reason,"updated_at":now})
            }
            Operation::ReadinessList { name, commit: sha } => {
                resource(&name)?;
                commit(&sha)?;
                let mut statement=tx.prepare("SELECT reviewer,reviewer_name,commit_sha,verdict,reason,updated FROM room_readiness WHERE room=?1 AND name=?2 ORDER BY reviewer,principal")?;
                let records=statement.query_map(params![room,name],|r|{let recorded:String=r.get(2)?;Ok(json!({"reviewer_id":r.get::<_,String>(0)?,"reviewer_name":r.get::<_,String>(1)?,"commit":recorded,"stale":recorded!=sha,"verdict":r.get::<_,String>(3)?,"reason":r.get::<_,String>(4)?,"updated_at":r.get::<_,i64>(5)?}))})?.collect::<Result<Vec<_>,_>>()?;
                json!({"name":name,"commit":sha,"records":records})
            }
            operation => {
                let name = match &operation {
                    Operation::Get { name }
                    | Operation::Acquire { name, .. }
                    | Operation::Heartbeat { name, .. }
                    | Operation::Release { name, .. }
                    | Operation::Validate { name, .. } => name,
                    _ => unreachable!(),
                };
                resource(name)?;
                let current = tx
                    .query_row(
                        &format!("SELECT {COLUMNS} FROM room_leases WHERE room=?1 AND name=?2"),
                        params![room, name],
                        |r| row(r, now),
                    )
                    .optional()?;
                match operation {
                    Operation::Get { .. } => json!({"lease":current.map(|r|r.lease)}),
                    Operation::Acquire {
                        name,
                        expected_generation,
                    } => {
                        if expected_generation < 0 {
                            return Err(CoordinationError::Invalid("negative generation"));
                        }
                        let generation = current.as_ref().map_or(0, |r| r.lease.generation);
                        // A retry of the same grant is idempotent, but never extends its lifetime.
                        let retry = current.as_ref().is_some_and(|r| {
                            r.principal == principal
                                && r.lease.holder_id == agent
                                && expected_generation.checked_add(1) == Some(generation)
                                && matches!(r.lease.status.as_str(), "active" | "overdue")
                        });
                        if !retry {
                            if generation != expected_generation {
                                return Err(CoordinationError::Conflict);
                            }
                            if current.as_ref().is_some_and(|r| {
                                matches!(r.lease.status.as_str(), "active" | "overdue")
                            }) {
                                return Err(CoordinationError::Conflict);
                            }
                            let next = generation
                                .checked_add(1)
                                .ok_or(CoordinationError::Invalid("generation exhausted"))?;
                            if current.is_none() {
                                let count: i64 = tx.query_row(
                                    "SELECT count(*) FROM room_leases WHERE room=?1",
                                    [room],
                                    |r| r.get(0),
                                )?;
                                if count >= MAX_RESOURCES {
                                    return Err(CoordinationError::Invalid(
                                        "room lease resource limit reached",
                                    ));
                                }
                            }
                            tx.execute("INSERT INTO room_leases(room,name,generation,principal,holder,holder_name,state,acquired,heartbeat,progressed) VALUES (?1,?2,?3,?4,?5,?6,'working',?7,?7,?7)
                                ON CONFLICT(room,name) DO UPDATE SET generation=excluded.generation,principal=excluded.principal,holder=excluded.holder,holder_name=excluded.holder_name,state='working',detail=NULL,progress=NULL,acquired=excluded.acquired,heartbeat=excluded.heartbeat,progressed=excluded.progressed,released=NULL",
                                params![room,name,next,principal,agent,agent_name,now])?;
                        }
                        let record = tx.query_row(
                            &format!("SELECT {COLUMNS} FROM room_leases WHERE room=?1 AND name=?2"),
                            params![room, name],
                            |r| row(r, now),
                        )?;
                        json!({"lease":record.lease})
                    }
                    operation => {
                        let (name, generation) = match &operation {
                            Operation::Heartbeat {
                                name, generation, ..
                            }
                            | Operation::Release { name, generation }
                            | Operation::Validate {
                                name, generation, ..
                            } => (name.clone(), *generation),
                            _ => unreachable!(),
                        };
                        let record = current.ok_or(CoordinationError::Lost)?;
                        if record.lease.generation != generation {
                            return Err(CoordinationError::Lost);
                        }
                        let validating = matches!(operation, Operation::Validate { .. });
                        match &operation {
                            Operation::Validate { holder, .. }
                                if holder != &record.lease.holder_id =>
                            {
                                return Err(CoordinationError::Lost)
                            }
                            Operation::Validate { .. } => {}
                            _ if record.principal != principal
                                || record.lease.holder_id != agent =>
                            {
                                return Err(CoordinationError::Lost)
                            }
                            _ => {}
                        }
                        if matches!(operation, Operation::Release { .. })
                            && record.lease.released_at.is_some()
                        {
                            json!({"lease":record.lease})
                        } else {
                            if !matches!(record.lease.status.as_str(), "active" | "overdue") {
                                return Err(CoordinationError::Lost);
                            }
                            match operation {
                                Operation::Heartbeat {
                                    state,
                                    detail,
                                    progress,
                                    ..
                                } => {
                                    if detail.as_ref().is_some_and(|v| !bounded(v, 512))
                                        || progress.as_ref().is_some_and(|v| !bounded(v, 512))
                                    {
                                        return Err(CoordinationError::Invalid("detail/progress exceeds 512 bytes or contains controls"));
                                    }
                                    let progressed = if progress
                                        .as_ref()
                                        .is_some_and(|p| Some(p) != record.lease.progress.as_ref())
                                    {
                                        now
                                    } else {
                                        record.lease.last_progress_at
                                    };
                                    // Preserve progress and explicit waiting state on liveness-only heartbeats.
                                    tx.execute("UPDATE room_leases SET heartbeat=MAX(heartbeat,?3),progressed=?4,state=?5,detail=?6,progress=?7 WHERE room=?1 AND name=?2",
                                        params![room,name,now,progressed,state_name(state.unwrap_or(record.lease.task_state)),detail.or(record.lease.detail),progress.or(record.lease.progress)])?;
                                }
                                Operation::Release { .. } => {
                                    tx.execute("UPDATE room_leases SET released=?3 WHERE room=?1 AND name=?2",params![room,name,now])?;
                                }
                                Operation::Validate { .. } => {}
                                _ => unreachable!(),
                            }
                            let record = tx.query_row(
                                &format!(
                                    "SELECT {COLUMNS} FROM room_leases WHERE room=?1 AND name=?2"
                                ),
                                params![room, record.lease.name],
                                |r| row(r, now),
                            )?;
                            if validating {
                                json!({"lease":record.lease,"valid":true})
                            } else {
                                json!({"lease":record.lease})
                            }
                        }
                    }
                }
            }
        };
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        for room in ["room", "other"] {
            s.create_room(room, room, None, None, None).unwrap();
        }
        s
    }
    fn call(s: &Store, op: Value, now: i64) -> Value {
        s.coordinate(
            "room",
            "principal",
            "bot",
            "Bot",
            serde_json::from_value(op).unwrap(),
            now,
        )
        .unwrap()
    }
    fn acquire(s: &Store, generation: i64, now: i64) -> Value {
        call(
            s,
            json!({"action":"acquire","name":"deploy:canyon","expected_generation":generation}),
            now,
        )
    }
    fn lease_op(action: &str, generation: i64) -> Value {
        let mut operation = json!({"action":action,"name":"deploy:canyon","generation":generation});
        if action == "validate" {
            operation["holder"] = "bot".into();
        }
        operation
    }
    fn lost(s: &Store, op: Value, now: i64) {
        assert!(matches!(
            s.coordinate(
                "room",
                "principal",
                "bot",
                "Bot",
                serde_json::from_value(op).unwrap(),
                now
            ),
            Err(CoordinationError::Lost)
        ));
    }
    #[test]
    fn timing_boundaries_and_progress_are_independent() {
        let s = store();
        acquire(&s, 0, 1000);
        let get = || json!({"action":"get","name":"deploy:canyon"});
        assert_eq!(call(&s, get(), 1299)["lease"]["status"], "active");
        assert_eq!(call(&s, get(), 1300)["lease"]["status"], "overdue");
        assert_eq!(call(&s, get(), 1420)["lease"]["status"], "expired");
        let mut heartbeat = lease_op("heartbeat", 1);
        heartbeat["state"] = "waiting_for_human".into();
        heartbeat["detail"] = "Approve deployment".into();
        call(&s, heartbeat, 1301);
        let renewed = call(&s, lease_op("heartbeat", 1), 1361);
        assert_eq!(renewed["lease"]["task_state"], "waiting_for_human");
        assert_eq!(renewed["lease"]["last_progress_at"], 1000);
        let mut heartbeat = lease_op("heartbeat", 1);
        heartbeat["progress"] = "Built candidate".into();
        assert_eq!(
            call(&s, heartbeat.clone(), 1421)["lease"]["last_progress_at"],
            1421
        );
        assert_eq!(call(&s, heartbeat, 1481)["lease"]["last_progress_at"], 1421);
        lost(&s, lease_op("heartbeat", 1), 1901);
    }
    #[test]
    fn fencing_owner_and_idempotence() {
        let s = store();
        acquire(&s, 0, 0);
        assert_eq!(acquire(&s, 0, 50)["lease"]["last_heartbeat_at"], 0);
        for (principal, agent) in [("intruder", "bot"), ("principal", "other")] {
            for action in ["heartbeat", "release"] {
                assert!(matches!(
                    s.coordinate(
                        "room",
                        principal,
                        agent,
                        "x",
                        serde_json::from_value(lease_op(action, 1)).unwrap(),
                        60
                    ),
                    Err(CoordinationError::Lost)
                ));
            }
        }
        let mut wrong_holder = lease_op("validate", 1);
        wrong_holder["holder"] = "other".into();
        lost(&s, wrong_holder, 60);
        assert_eq!(
            s.coordinate(
                "room",
                "executor",
                "gate",
                "Gate",
                serde_json::from_value(lease_op("validate", 1)).unwrap(),
                60
            )
            .unwrap()["valid"],
            true
        );
        call(&s, lease_op("release", 1), 61);
        call(&s, lease_op("release", 1), 62);
        assert_eq!(acquire(&s, 1, 63)["lease"]["generation"], 2);
        for action in ["heartbeat", "release", "validate"] {
            lost(&s, lease_op(action, 1), 64);
        }
        assert!(s
            .coordinate(
                "room",
                "principal",
                "bot",
                "Bot",
                Operation::Acquire {
                    name: "deploy:canyon".into(),
                    expected_generation: 0
                },
                500
            )
            .is_err());
        assert_eq!(acquire(&s, 2, 500)["lease"]["generation"], 3);
    }
    #[test]
    fn waiting_heartbeats_keep_ownership_but_stopped_host_expires() {
        let s = store();
        acquire(&s, 0, 0);
        let mut op = lease_op("heartbeat", 1);
        op["state"] = "waiting_for_human".into();
        call(&s, op, 60);
        for now in (120..=3600).step_by(60) {
            call(&s, lease_op("heartbeat", 1), now);
        }
        let value = call(&s, json!({"action":"get","name":"deploy:canyon"}), 3900);
        assert_eq!(value["lease"]["status"], "overdue");
        assert_eq!(value["lease"]["last_progress_at"], 0);
        lost(&s, lease_op("heartbeat", 1), 4020);
    }
    #[test]
    fn readiness_is_commit_specific_and_reviewer_scoped() {
        let s = store();
        let sha = "a".repeat(40);
        let other = "b".repeat(40);
        call(
            &s,
            json!({"action":"readiness","name":"promote","commit":sha,"verdict":"ready","reason":"Tests passed"}),
            0,
        );
        let list = |sha: &str| json!({"action":"readiness_list","name":"promote","commit":sha});
        assert_eq!(call(&s, list(&sha), 1)["records"][0]["stale"], false);
        assert_eq!(call(&s, list(&other), 1)["records"][0]["stale"], true);
        call(
            &s,
            json!({"action":"readiness","name":"promote","commit":sha,"verdict":"withdrawn","reason":"New finding"}),
            2,
        );
        assert_eq!(
            call(&s, list(&sha), 3)["records"][0]["verdict"],
            "withdrawn"
        );
        s.coordinate("room","second","bot","Bot",serde_json::from_value(json!({"action":"readiness","name":"promote","commit":other,"verdict":"blocked","reason":"Review pending"})).unwrap(),4).unwrap();
        assert_eq!(
            call(&s, list(&sha), 5)["records"].as_array().unwrap().len(),
            2
        );
    }
    #[test]
    fn concurrent_acquire_has_one_winner_and_room_isolation() {
        let s = std::sync::Arc::new(store());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = s.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    s.coordinate(
                        "room",
                        "p",
                        &i.to_string(),
                        "Bot",
                        Operation::Acquire {
                            name: "resource".into(),
                            expected_generation: 0,
                        },
                        0,
                    )
                    .is_ok()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .filter(|h| h.thread().id() != std::thread::current().id())
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
        let result = s
            .coordinate(
                "other",
                "p",
                "a",
                "A",
                Operation::Acquire {
                    name: "resource".into(),
                    expected_generation: 0,
                },
                0,
            )
            .unwrap();
        assert_eq!(result["lease"]["generation"], 1);
        let list = call(&s, json!({"action":"list"}), 0);
        assert!(list["leases"][0].get("token").is_none());
    }
    #[test]
    fn restart_preserves_generation_and_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let s = Store::open(&path).unwrap();
            s.create_room("room", "room", None, None, None).unwrap();
            acquire(&s, 0, 0);
        }
        let s = Store::open(&path).unwrap();
        call(&s, lease_op("heartbeat", 1), 60);
        assert_eq!(acquire(&s, 1, 480)["lease"]["generation"], 2);
    }
    #[test]
    fn destruction_removes_coordination_rows() {
        let s = Store::open_in_memory().unwrap();
        s.create_room("room", "Room", None, None, Some("bot"))
            .unwrap();
        acquire(&s, 0, 0);
        call(
            &s,
            json!({"action":"readiness","name":"promote","commit":"a".repeat(40),"verdict":"ready","reason":"ok"}),
            0,
        );
        s.destroy_room_authorized("room", "bot", "", true).unwrap();
        let db = s.conn.lock().unwrap();
        for table in ["room_leases", "room_readiness"] {
            let count: i64 = db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
    }
    #[test]
    fn listing_remains_under_frame_limit() {
        let s = store();
        for i in 0..MAX_RESOURCES {
            s.coordinate(
                "room",
                &format!("principal-{i}"),
                "bot",
                "Bot",
                Operation::Readiness {
                    name: "resource".into(),
                    commit: "a".repeat(40),
                    verdict: Verdict::Blocked,
                    reason: "\\".repeat(2048),
                },
                0,
            )
            .unwrap();
        }
        let denied = s.coordinate(
            "room",
            "p",
            "a",
            "A",
            Operation::Readiness {
                name: "one-more".into(),
                commit: "a".repeat(40),
                verdict: Verdict::Ready,
                reason: String::new(),
            },
            0,
        );
        assert!(denied.is_err());
        let records = call(
            &s,
            json!({"action":"readiness_list","name":"resource","commit":"a".repeat(40)}),
            0,
        );
        assert_eq!(
            records["records"].as_array().unwrap().len(),
            MAX_RESOURCES as usize
        );
        assert!(serde_json::to_vec(&records).unwrap().len() < 1_000_000);
    }
    #[test]
    fn rejects_unknown_fields_and_invalid_inputs() {
        assert!(
            serde_json::from_value::<Operation>(json!({"action":"list","token":"secret"})).is_err()
        );
        let s = store();
        for name in ["", " padded", "control\n"] {
            assert!(s
                .coordinate(
                    "room",
                    "p",
                    "a",
                    "A",
                    Operation::Get { name: name.into() },
                    0
                )
                .is_err());
        }
        assert!(commit("main").is_err());
        assert!(commit(&"A".repeat(40)).is_err());
    }
}
