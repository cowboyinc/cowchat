use super::Store;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const PREFIX: &str = "csoc_";
pub const OWNER_PREFIX: &str = "\0cowchat:social:";

#[derive(Clone, Debug)]
pub struct Principal {
    pub account_id: String,
    pub agent_id: Option<String>,
}

pub fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub fn principal(conn: &Connection, key: &str) -> rusqlite::Result<Option<Principal>> {
    if !key.starts_with(PREFIX) {
        return Ok(None);
    }
    conn.query_row("SELECT account_id,agent_id FROM social_credentials WHERE digest=?1 AND revoked=0 AND expires>=unixepoch()", [hash(key)], |r| Ok(Principal {account_id:r.get(0)?,agent_id:r.get(1)?})).optional()
}
pub fn room_access(conn: &Connection, room: &str, key: &str) -> rusqlite::Result<Option<bool>> {
    let managed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_rooms WHERE room_id=?1)",
        [room],
        |r| r.get(0),
    )?;
    if !managed {
        return Ok(key.starts_with(PREFIX).then_some(false));
    }
    let Some(p) = principal(conn, key)? else {
        return Ok(Some(false));
    };
    let allowed = if let Some(agent) = p.agent_id {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM social_admissions a JOIN social_members m ON m.room_id=a.room_id AND m.account_id=?3 JOIN social_agents g ON g.id=a.agent_id WHERE a.room_id=?1 AND a.agent_id=?2 AND g.revoked=0 AND g.owner_id=?3)", params![room,agent,p.account_id],|r|r.get(0))?
    } else {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM social_members WHERE room_id=?1 AND account_id=?2)",
            params![room, p.account_id],
            |r| r.get(0),
        )?
    };
    Ok(Some(allowed))
}
pub fn room_owner(conn: &Connection, room: &str, key: &str) -> rusqlite::Result<Option<bool>> {
    let owner: Option<String> = conn
        .query_row(
            "SELECT owner_id FROM social_rooms WHERE room_id=?1",
            [room],
            |r| r.get(0),
        )
        .optional()?;
    match owner {
        Some(owner) => {
            Ok(Some(principal(conn, key)?.is_some_and(|p| {
                p.agent_id.is_none() && p.account_id == owner
            })))
        }
        None => Ok(key.starts_with(PREFIX).then_some(false)),
    }
}
fn account(conn: &Connection, id: &str) -> anyhow::Result<Value> {
    Ok(conn.query_row("SELECT id,handle,display_name FROM social_accounts WHERE id=?1",[id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"handle":r.get::<_,String>(1)?,"display_name":r.get::<_,String>(2)?})))?)
}
fn valid_handle(handle: &str) -> bool {
    (3..=40).contains(&handle.len())
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn available_handle(conn: &Connection, handle: &str, account: &str) -> anyhow::Result<bool> {
    Ok(!conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_accounts WHERE handle=?1 COLLATE NOCASE AND id<>?2)",
        params![handle, account],
        |r| r.get::<_, bool>(0),
    )?)
}
fn validate_profile(
    conn: &Connection,
    account: &str,
    handle: &str,
    name: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        valid_handle(handle)
            && !name.is_empty()
            && name.chars().count() <= 100
            && name.chars().all(display_char),
        "invalid profile"
    );
    anyhow::ensure!(
        available_handle(conn, handle, account)?,
        "handle_unavailable"
    );
    Ok(())
}
fn link_target(conn: &Connection, slug: &str) -> anyhow::Result<String> {
    anyhow::ensure!(valid_handle(slug), "not found");
    conn.query_row("SELECT a.id FROM social_friend_links l JOIN social_accounts a ON a.id=l.account_id WHERE l.slug=?1 COLLATE NOCASE",[slug],|r|r.get(0)).optional()?.ok_or_else(||anyhow::anyhow!("not found"))
}
fn relationship(
    conn: &Connection,
    from: &str,
    to: &str,
) -> anyhow::Result<(&'static str, Option<String>)> {
    if from == to {
        return Ok(("self", None));
    }
    if conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_friends WHERE a=?1 AND b=?2)",
        params![from, to],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(("friends", None));
    }
    for (a, b, status) in [
        (from, to, "outgoing_pending"),
        (to, from, "incoming_pending"),
    ] {
        if let Some(id) = conn.query_row("SELECT id FROM social_friend_requests WHERE from_id=?1 AND to_id=?2 AND status='pending'",params![a,b],|r|r.get::<_,String>(0)).optional()? { return Ok((status,Some(id))); }
    }
    Ok(("none", None))
}
fn request_friend(conn: &Connection, from: &str, to: &str) -> anyhow::Result<Value> {
    let (status, pending) = relationship(conn, from, to)?;
    anyhow::ensure!(status != "self", "invalid recipient");
    anyhow::ensure!(status != "friends", "already friends");
    anyhow::ensure!(status != "incoming_pending", "incoming_pending");
    if let Some(id) = pending {
        return Ok(json!({"request":friend_request(conn,&id)?,"relationship":status}));
    }
    bounded(
        conn,
        "SELECT COUNT(*) FROM social_friend_requests WHERE from_id=?1 AND status='pending'",
        from,
        100,
    )?;
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO social_friend_requests(id,from_id,to_id) VALUES(?1,?2,?3)",
        params![id, from, to],
    )?;
    Ok(json!({"request":friend_request(conn,&id)?,"relationship":"outgoing_pending"}))
}
fn default_handle(conn: &Connection, name: &str) -> rusqlite::Result<String> {
    let mut base = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    base.truncate(40);
    base = base.trim_end_matches('-').to_owned();
    if base.len() < 3 {
        base = "Cowboy".to_owned();
    }
    let mut handle = base.clone();
    let mut suffix = 0u64;
    while conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_accounts WHERE handle=?1 COLLATE NOCASE)",
        [&handle],
        |row| row.get::<_, bool>(0),
    )? {
        let ending = format!("-{suffix}");
        let stem = base[..base.len().min(40 - ending.len())].trim_end_matches('-');
        handle = format!("{stem}{ending}");
        suffix += 1;
    }
    Ok(handle)
}
fn starter_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            stem.push(c);
        } else if !stem.is_empty() && !stem.ends_with('_') {
            stem.push('_');
        }
        if stem.chars().count() >= 70 {
            break;
        }
    }
    let stem = stem.trim_end_matches('_');
    if stem.is_empty() {
        "Cowboy".to_owned()
    } else {
        stem.to_owned()
    }
}
fn setup(conn: &Connection, account: &str) -> anyhow::Result<Value> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT agent_id,agent_name,room_id,room_name FROM social_setup WHERE account_id=?1",
            [account],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((agent_id, agent_name, room_id, room_name)) = row else {
        return Ok(json!({"setup":null}));
    };
    let agent = agents(conn, Some(account), None, false)?
        .into_iter()
        .find(|g| g["id"] == agent_id);
    let room: Option<Value> = conn.query_row(
        "SELECT r.room_id,r.name,s.owner_id FROM rooms r JOIN social_rooms s ON s.room_id=r.room_id JOIN social_members m ON m.room_id=r.room_id AND m.account_id=?2 WHERE r.room_id=?1",
        params![room_id,account], |r| Ok(json!({"room_id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"owner_id":r.get::<_,String>(2)?}))).optional()?;
    let admitted = agent.is_some()
        && room.is_some()
        && conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM social_admissions WHERE room_id=?1 AND agent_id=?2)",
            params![room_id, agent_id],
            |r| r.get::<_, bool>(0),
        )?;
    Ok(
        json!({"setup":{"agent_id":agent_id,"agent_name":agent_name,"room_id":room_id,"room_name":room_name,"agent":agent,"room":room,"bot_admitted":admitted}}),
    )
}
fn agents(
    conn: &Connection,
    owner: Option<&str>,
    room: Option<&str>,
    discoverable: bool,
) -> anyhow::Result<Vec<Value>> {
    let mut s=conn.prepare("SELECT id,owner_id,name,discoverable FROM social_agents WHERE revoked=0 AND (?1 IS NULL OR owner_id=?1) AND (?2 IS NULL OR id IN (SELECT agent_id FROM social_admissions WHERE room_id=?2)) AND (?3=0 OR discoverable=1) ORDER BY name,id")?;
    let rows=s.query_map(params![owner,room,discoverable],|r|Ok(json!({"id":r.get::<_,String>(0)?,"owner_id":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"discoverable":r.get::<_,bool>(3)?})))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
fn members(conn: &Connection, room: &str) -> anyhow::Result<Vec<Value>> {
    let mut s =
        conn.prepare("SELECT account_id FROM social_members WHERE room_id=?1 ORDER BY account_id")?;
    let ids = s
        .query_map([room], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| account(conn, id)).collect()
}
fn string<'a>(body: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    body.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("invalid {key}"))
}
fn ids(body: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    match body.get(key) {
        None => Ok(vec![]),
        Some(Value::Array(a)) if a.len() <= 100 => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("invalid {key}"))
            })
            .collect(),
        _ => anyhow::bail!("invalid {key}"),
    }
}
fn approve(conn: &Connection, room: &str, owner: &str, ids: &[String]) -> anyhow::Result<()> {
    for id in ids {
        let owned: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM social_agents WHERE id=?1 AND owner_id=?2 AND revoked=0)",
            params![id, owner],
            |r| r.get(0),
        )?;
        anyhow::ensure!(owned, "not found");
    }
    for id in ids {
        conn.execute(
            "INSERT OR IGNORE INTO social_admissions(room_id,agent_id) VALUES(?1,?2)",
            params![room, id],
        )?;
    }
    Ok(())
}
fn invitation(conn: &Connection, id: &str) -> anyhow::Result<Value> {
    let (room,from,to,proposed,status,created):(String,String,String,String,String,String)=conn.query_row("SELECT room_id,from_id,to_id,proposed,status,created_at FROM social_room_invitations WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let name: String = conn.query_row("SELECT name FROM rooms WHERE room_id=?1", [&room], |r| {
        r.get(0)
    })?;
    let wanted: Vec<String> = serde_json::from_str(&proposed)?;
    let proposed = agents(conn, Some(&to), None, true)?
        .into_iter()
        .filter(|a| wanted.iter().any(|id| a["id"] == *id))
        .collect::<Vec<_>>();
    Ok(
        json!({"id":id,"room_id":room,"room_name":name,"from":account(conn,&from)?,"to":account(conn,&to)?,"proposed_agents":proposed,"members":members(conn,&room)?,"agents":agents(conn,None,Some(&room),false)?,"history_policy":"full","encrypted":false,"slack_connected":false,"status":status,"created_at":created}),
    )
}
fn invite(
    conn: &Connection,
    room: &str,
    from: &str,
    to: &str,
    proposed: Vec<String>,
) -> anyhow::Result<Value> {
    let friends: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_friends WHERE a=?1 AND b=?2)",
        params![from, to],
        |r| r.get(0),
    )?;
    anyhow::ensure!(friends, "not found");
    let visible = agents(conn, Some(to), None, true)?;
    anyhow::ensure!(
        proposed
            .iter()
            .all(|id| visible.iter().any(|a| a["id"] == *id)),
        "not found"
    );
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute("INSERT INTO social_room_invitations(id,room_id,from_id,to_id,proposed) VALUES(?1,?2,?3,?4,?5)",params![id,room,from,to,serde_json::to_string(&proposed)?])?;
    invitation(conn, &id)
}
impl Store {
    pub(super) fn initialize_social(&self) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute_batch("CREATE TABLE IF NOT EXISTS social_accounts(id TEXT PRIMARY KEY,handle TEXT NOT NULL UNIQUE,display_name TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS social_accounts_handle_nocase ON social_accounts(handle COLLATE NOCASE);
CREATE TABLE IF NOT EXISTS social_friend_links(account_id TEXT PRIMARY KEY,slug TEXT NOT NULL COLLATE NOCASE UNIQUE);
CREATE TABLE IF NOT EXISTS social_identities(provider TEXT NOT NULL,subject TEXT NOT NULL,account_id TEXT NOT NULL,label TEXT NOT NULL,PRIMARY KEY(provider,subject));
CREATE TABLE IF NOT EXISTS social_credentials(digest TEXT PRIMARY KEY,account_id TEXT NOT NULL,agent_id TEXT,revoked INTEGER NOT NULL DEFAULT 0,expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS social_oauth(request_hash TEXT PRIMARY KEY,state_hash TEXT NOT NULL UNIQUE,provider TEXT NOT NULL,challenge TEXT NOT NULL,link_account TEXT,link_digest TEXT,callback_port INTEGER NOT NULL,source_hash TEXT NOT NULL,completion_hash TEXT,provider_subject TEXT,provider_label TEXT,expires INTEGER NOT NULL,claimed INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS social_setup(account_id TEXT PRIMARY KEY,agent_id TEXT NOT NULL,agent_name TEXT NOT NULL,room_id TEXT NOT NULL,room_name TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS social_agents(id TEXT PRIMARY KEY,owner_id TEXT NOT NULL,name TEXT NOT NULL,discoverable INTEGER NOT NULL,revoked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS social_friend_requests(id TEXT PRIMARY KEY,from_id TEXT NOT NULL,to_id TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
CREATE UNIQUE INDEX IF NOT EXISTS social_pending_friend ON social_friend_requests(from_id,to_id) WHERE status='pending';
CREATE TABLE IF NOT EXISTS social_friends(a TEXT NOT NULL,b TEXT NOT NULL,PRIMARY KEY(a,b));
CREATE TABLE IF NOT EXISTS social_rooms(room_id TEXT PRIMARY KEY,owner_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS social_members(room_id TEXT NOT NULL,account_id TEXT NOT NULL,PRIMARY KEY(room_id,account_id));
CREATE TABLE IF NOT EXISTS social_admissions(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,PRIMARY KEY(room_id,agent_id));
CREATE TABLE IF NOT EXISTS social_room_invitations(id TEXT PRIMARY KEY,room_id TEXT NOT NULL,from_id TEXT NOT NULL,to_id TEXT NOT NULL,proposed TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);")
    }
    pub fn social_friend_link_account(&self, slug: &str) -> anyhow::Result<Value> {
        let conn = self.conn.lock().unwrap();
        let target = link_target(&conn, slug)?;
        account(&conn, &target)
    }
    pub fn social_principal(&self, key: &str) -> Option<Principal> {
        principal(&self.conn.lock().unwrap(), key).ok().flatten()
    }
    pub fn social_room_access(&self, room: &str, key: &str) -> Option<bool> {
        room_access(&self.conn.lock().unwrap(), room, key).unwrap_or(Some(false))
    }
    pub fn social_room_owner(&self, room: &str, key: &str) -> Option<bool> {
        room_owner(&self.conn.lock().unwrap(), room, key).unwrap_or(Some(false))
    }
    pub fn social_identity_allowed(&self, id: &str, key: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        let assigned: Option<String> = conn
            .query_row("SELECT id FROM social_agents WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .ok()
            .flatten();
        match principal(&conn,key).ok().flatten() {Some(p)=>match p.agent_id {Some(agent)=>agent==id,None=>assigned.is_none()},None=>!key.starts_with(PREFIX)&&assigned.is_none()&&!conn.query_row("SELECT EXISTS(SELECT 1 FROM agent_identities WHERE agent_id=?1 AND owner_key LIKE 'social-account:%')",[id],|r|r.get::<_,bool>(0)).unwrap_or(true)}
    }
    pub fn social_display_name(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        let p = principal(&conn, key).ok().flatten()?;
        let (handle, display): (String, String) = conn
            .query_row(
                "SELECT handle,display_name FROM social_accounts WHERE id=?1",
                [&p.account_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        let name = if let Some(agent) = p.agent_id {
            conn.query_row(
                "SELECT name FROM social_agents WHERE id=?1 AND revoked=0",
                [agent],
                |r| r.get::<_, String>(0),
            )
            .ok()?
        } else {
            display
        };
        Some(format!("{name} · @{handle}"))
    }
    pub fn social_action(
        &self,
        key: &str,
        method: &str,
        path: &str,
        body: &Value,
    ) -> anyhow::Result<Value> {
        let mut conn = self.conn.lock().unwrap();
        let p = principal(&conn, key)?.ok_or_else(|| anyhow::anyhow!("unauthorized"))?;
        anyhow::ensure!(p.agent_id.is_none(), "human approval required");
        let tx = conn.transaction()?;
        let a = &p.account_id;
        let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
        let response = match (method, parts.as_slice()) {
            ("POST", ["handles", "availability"]) => {
                let handle = string(body, "handle")?.trim();
                let valid = valid_handle(handle);
                json!({"handle":handle,"valid":valid,"available":valid && available_handle(&tx,handle,a)?})
            }
            ("POST", ["me", "friend-link"]) => {
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT slug FROM social_friend_links WHERE account_id=?1",
                        [a],
                        |r| r.get(0),
                    )
                    .optional()?;
                let slug = if let Some(existing) = existing {
                    existing
                } else {
                    let base: String =
                        tx.query_row("SELECT handle FROM social_accounts WHERE id=?1", [a], |r| {
                            r.get(0)
                        })?;
                    let mut slug = base.clone();
                    let mut suffix = 0u64;
                    while tx.query_row("SELECT EXISTS(SELECT 1 FROM social_friend_links WHERE slug=?1 COLLATE NOCASE)",[&slug],|r|r.get::<_,bool>(0))? {
                        let ending = format!("-{suffix}");
                        slug = format!("{}{}",&base[..base.len().min(40-ending.len())],ending);
                        suffix += 1;
                    }
                    tx.execute(
                        "INSERT INTO social_friend_links(account_id,slug) VALUES(?1,?2)",
                        params![a, slug],
                    )?;
                    slug
                };
                json!({"slug":slug})
            }
            ("GET", ["friend-links", slug]) => {
                let to = link_target(&tx, slug)?;
                let (status, request_id) = relationship(&tx, a, &to)?;
                json!({"account":account(&tx,&to)?,"relationship":status,"request_id":request_id})
            }
            ("POST", ["friend-links", slug, "request"]) => {
                let to = link_target(&tx, slug)?;
                request_friend(&tx, a, &to)?
            }
            ("GET", ["setup"]) => setup(&tx, a)?,
            ("POST", ["setup"]) => {
                let established = setup(&tx, a)?;
                if established["setup"].is_null() {
                    let name = string(body, "display_name")?.trim();
                    anyhow::ensure!(
                        !name.is_empty()
                            && name.chars().count() <= 100
                            && name.chars().all(display_char),
                        "invalid display_name"
                    );
                    if body.get("handle").is_some() {
                        let handle = string(body, "handle")?.trim();
                        validate_profile(&tx, a, handle, name)?;
                        tx.execute(
                            "UPDATE social_accounts SET handle=?1 WHERE id=?2",
                            params![handle, a],
                        )?;
                    }
                    bounded(
                        &tx,
                        "SELECT COUNT(*) FROM social_agents WHERE owner_id=?1 AND revoked=0",
                        a,
                        100,
                    )?;
                    bounded(
                        &tx,
                        "SELECT COUNT(*) FROM social_rooms WHERE owner_id=?1",
                        a,
                        100,
                    )?;
                    let stem = starter_stem(name);
                    let agent_name = format!("{stem}_bot");
                    let base = format!("{stem}_Home");
                    let mut room_name = base.clone();
                    let mut suffix = 0u64;
                    while tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM rooms WHERE name=?1)",
                        [&room_name],
                        |r| r.get::<_, bool>(0),
                    )? {
                        room_name = format!("{base}_{suffix}");
                        suffix += 1;
                    }
                    let agent = uuid::Uuid::new_v4().to_string();
                    let room = uuid::Uuid::new_v4().to_string();
                    tx.execute(
                        "UPDATE social_accounts SET display_name=?1 WHERE id=?2",
                        params![name, a],
                    )?;
                    tx.execute("INSERT INTO social_agents(id,owner_id,name,discoverable) VALUES(?1,?2,?3,0)",params![agent,a,agent_name])?;
                    tx.execute("INSERT INTO rooms(room_id,name,visibility,owner_key,encrypted,created_by) VALUES(?1,?2,'private',?3,0,?4)",params![room,room_name,format!("{OWNER_PREFIX}{a}"),a])?;
                    tx.execute(
                        "INSERT INTO social_rooms(room_id,owner_id) VALUES(?1,?2)",
                        params![room, a],
                    )?;
                    tx.execute(
                        "INSERT INTO social_members(room_id,account_id) VALUES(?1,?2)",
                        params![room, a],
                    )?;
                    approve(&tx, &room, a, std::slice::from_ref(&agent))?;
                    tx.execute("INSERT INTO social_setup(account_id,agent_id,agent_name,room_id,room_name) VALUES(?1,?2,?3,?4,?5)",params![a,agent,agent_name,room,room_name])?;
                    setup(&tx, a)?
                } else {
                    established
                }
            }
            ("GET", ["me"]) | ("PATCH", ["me"]) => {
                if method == "PATCH" {
                    let handle = string(body, "handle")?.trim();
                    let name = string(body, "display_name")?.trim();
                    validate_profile(&tx, a, handle, name)?;
                    tx.execute(
                        "UPDATE social_accounts SET handle=?1,display_name=?2 WHERE id=?3",
                        params![handle, name, a],
                    )?;
                }
                let mut s=tx.prepare("SELECT provider,subject,label FROM social_identities WHERE account_id=?1 ORDER BY provider")?;
                let identities=s.query_map([a],|r|Ok(json!({"provider":r.get::<_,String>(0)?,"subject":r.get::<_,String>(1)?,"label":r.get::<_,String>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
                json!({"account":account(&tx,a)?,"identities":identities})
            }
            ("POST", ["logout"]) => {
                tx.execute(
                    "UPDATE social_credentials SET revoked=1 WHERE digest=?1",
                    [hash(key)],
                )?;
                json!({"ok":true})
            }
            ("GET", ["friends"]) => {
                let mut s = tx.prepare("SELECT b FROM social_friends WHERE a=?1 ORDER BY b")?;
                let friends=s.query_map([a],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?.iter().map(|id|Ok(json!({"account":account(&tx,id)?,"agents":agents(&tx,Some(id),None,true)?}))).collect::<anyhow::Result<Vec<_>>>()?;
                json!({"friends":friends})
            }
            ("DELETE", ["friends", other]) => {
                tx.execute(
                    "DELETE FROM social_friends WHERE (a=?1 AND b=?2) OR (a=?2 AND b=?1)",
                    params![a, other],
                )?;
                tx.execute("UPDATE social_friend_requests SET status='declined' WHERE status='pending' AND ((from_id=?1 AND to_id=?2) OR (from_id=?2 AND to_id=?1))",params![a,other])?;
                tx.execute("UPDATE social_room_invitations SET status='declined' WHERE status='pending' AND ((from_id=?1 AND to_id=?2) OR (from_id=?2 AND to_id=?1))",params![a,other])?;
                json!({"ok":true})
            }
            ("POST", ["friend-requests"]) => {
                let to: String = tx
                    .query_row(
                        "SELECT id FROM social_accounts WHERE handle=?1 COLLATE NOCASE",
                        [string(body, "handle")?.trim()],
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| anyhow::anyhow!("not found"))?;
                request_friend(&tx, a, &to)?
            }
            ("GET", ["friend-requests"]) => {
                let mut s=tx.prepare("SELECT id FROM social_friend_requests WHERE from_id=?1 OR to_id=?1 ORDER BY created_at DESC,id")?;
                let values = s
                    .query_map([a], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .map(|id| friend_request(&tx, id))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                json!({"requests":values})
            }
            ("POST", ["friend-requests", id, action])
                if *action == "accept" || *action == "decline" =>
            {
                let from:String=tx.query_row("SELECT from_id FROM social_friend_requests WHERE id=?1 AND to_id=?2 AND status='pending'",params![id,a],|r|r.get(0)).map_err(|_|anyhow::anyhow!("not found"))?;
                let status = if *action == "accept" {
                    "accepted"
                } else {
                    "declined"
                };
                tx.execute(
                    "UPDATE social_friend_requests SET status=?1 WHERE id=?2",
                    params![status, id],
                )?;
                if *action == "accept" {
                    tx.execute(
                        "INSERT OR IGNORE INTO social_friends(a,b) VALUES(?1,?2),(?2,?1)",
                        params![a, from],
                    )?;
                }
                json!({"ok":true})
            }
            ("GET", ["agents"]) => json!({"agents":agents(&tx,Some(a),None,false)?}),
            ("POST", ["agents"]) => {
                bounded(
                    &tx,
                    "SELECT COUNT(*) FROM social_agents WHERE owner_id=?1 AND revoked=0",
                    a,
                    100,
                )?;
                let default_name = format!(
                    "{}_bot",
                    starter_stem(account(&tx, a)?["display_name"].as_str().unwrap())
                );
                let name = if body.get("name").is_none() {
                    default_name.as_str()
                } else {
                    string(body, "name")?.trim()
                };
                anyhow::ensure!(
                    !name.is_empty()
                        && name.chars().count() <= 100
                        && name.chars().all(display_char),
                    "invalid name"
                );
                let id = uuid::Uuid::new_v4().to_string();
                let discoverable = match body.get("discoverable") {
                    None => true,
                    Some(value) => value
                        .as_bool()
                        .ok_or_else(|| anyhow::anyhow!("invalid discoverable"))?,
                };
                tx.execute(
                    "INSERT INTO social_agents(id,owner_id,name,discoverable) VALUES(?1,?2,?3,?4)",
                    params![id, a, name, discoverable],
                )?;
                let key = credential(&tx, a, Some(&id))?;
                json!({"agent":{"id":id,"owner_id":a,"name":name,"discoverable":discoverable},"api_key":key})
            }
            ("POST", ["agents", id, "credentials"]) => {
                let agent = agents(&tx, Some(a), None, false)?
                    .into_iter()
                    .find(|g| g["id"] == *id)
                    .ok_or_else(|| anyhow::anyhow!("not found"))?;
                let active: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM social_credentials WHERE agent_id=?1 AND revoked=0 AND expires>=unixepoch()",
                    [id], |row| row.get(0),
                )?;
                anyhow::ensure!(active < 20, "credential limit reached");
                let key = credential(&tx, a, Some(id))?;
                json!({"agent":agent,"api_key":key})
            }
            ("DELETE", ["agents", id]) => {
                let n = tx.execute(
                    "UPDATE social_agents SET revoked=1 WHERE id=?1 AND owner_id=?2",
                    params![id, a],
                )?;
                anyhow::ensure!(n > 0, "not found");
                tx.execute(
                    "UPDATE social_credentials SET revoked=1 WHERE agent_id=?1",
                    [id],
                )?;
                tx.execute("DELETE FROM social_admissions WHERE agent_id=?1", [id])?;
                json!({"ok":true})
            }
            ("POST", ["rooms"]) => {
                bounded(
                    &tx,
                    "SELECT COUNT(*) FROM social_rooms WHERE owner_id=?1",
                    a,
                    100,
                )?;
                let name = super::normalize_room_name(string(body, "name")?)
                    .map_err(anyhow::Error::msg)?;
                let friend = match body.get("friend_id") {
                    None => None,
                    Some(Value::String(id)) if !id.trim().is_empty() => Some(id.as_str()),
                    _ => anyhow::bail!("invalid friend_id"),
                };
                let proposed = ids(body, "proposed_agent_ids")?;
                anyhow::ensure!(
                    friend.is_some() || proposed.is_empty(),
                    "proposed_agent_ids requires friend_id"
                );
                let room = uuid::Uuid::new_v4().to_string();
                tx.execute("INSERT INTO rooms(room_id,name,visibility,owner_key,encrypted,created_by) VALUES(?1,?2,'private',?3,0,?4)",params![room,name,format!("{OWNER_PREFIX}{a}"),a])?;
                tx.execute(
                    "INSERT INTO social_rooms(room_id,owner_id) VALUES(?1,?2)",
                    params![room, a],
                )?;
                tx.execute(
                    "INSERT INTO social_members(room_id,account_id) VALUES(?1,?2)",
                    params![room, a],
                )?;
                approve(&tx, &room, a, &ids(body, "agent_ids")?)?;
                let invitation = friend
                    .map(|friend| invite(&tx, &room, a, friend, proposed))
                    .transpose()?;
                json!({"room_id":room,"invitation":invitation})
            }
            ("GET", ["rooms"]) => {
                let mut s=tx.prepare("SELECT r.room_id,r.name,s.owner_id FROM rooms r JOIN social_rooms s ON s.room_id=r.room_id JOIN social_members m ON m.room_id=r.room_id WHERE m.account_id=?1 ORDER BY r.name")?;
                let rooms=s.query_map([a],|r|Ok(json!({"room_id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"owner_id":r.get::<_,String>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
                json!({"rooms":rooms})
            }
            ("GET", ["room-invitations"]) => {
                let mut s=tx.prepare("SELECT id FROM social_room_invitations WHERE status='pending' AND (to_id=?1 OR from_id=?1) ORDER BY created_at DESC,id")?;
                let values = s
                    .query_map([a], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .map(|id| invitation(&tx, id))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                json!({"invitations":values})
            }
            ("POST", ["room-invitations", id, action])
                if *action == "accept" || *action == "decline" =>
            {
                let room:String=tx.query_row("SELECT room_id FROM social_room_invitations WHERE id=?1 AND to_id=?2 AND status='pending'",params![id,a],|r|r.get(0)).map_err(|_|anyhow::anyhow!("not found"))?;
                if *action == "accept" {
                    let still_friends:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM social_friends f JOIN social_room_invitations i ON i.from_id=f.a AND i.to_id=f.b WHERE i.id=?1)",[id],|r|r.get(0))?;
                    anyhow::ensure!(still_friends, "not found");
                    tx.execute(
                        "INSERT OR IGNORE INTO social_members(room_id,account_id) VALUES(?1,?2)",
                        params![room, a],
                    )?;
                    approve(&tx, &room, a, &ids(body, "agent_ids")?)?;
                }
                tx.execute(
                    "UPDATE social_room_invitations SET status=?1 WHERE id=?2",
                    params![
                        if *action == "accept" {
                            "accepted"
                        } else {
                            "declined"
                        },
                        id
                    ],
                )?;
                if *action == "accept" {
                    json!({"room_id":room})
                } else {
                    json!({"ok":true})
                }
            }
            ("GET", ["rooms", room, "participants"]) => {
                anyhow::ensure!(room_access(&tx, room, key)? == Some(true), "not found");
                let (name,owner):(String,String)=tx.query_row("SELECT r.name,s.owner_id FROM rooms r JOIN social_rooms s ON s.room_id=r.room_id WHERE r.room_id=?1",[room],|r|Ok((r.get(0)?,r.get(1)?)))?;
                json!({"room_id":room,"name":name,"owner_id":owner,"history_policy":"full","encrypted":false,"slack_connected":false,"members":members(&tx,room)?,"agents":agents(&tx,None,Some(room),false)?})
            }
            ("POST", ["rooms", _, "invitations"]) => {
                anyhow::bail!("Additional people are not supported in v1")
            }
            ("POST", ["rooms", room, "agents"]) => {
                anyhow::ensure!(room_access(&tx, room, key)? == Some(true), "not found");
                approve(&tx, room, a, &ids(body, "agent_ids")?)?;
                json!({"ok":true})
            }
            ("DELETE", ["rooms", room, "agents", agent]) => {
                anyhow::ensure!(room_access(&tx, room, key)? == Some(true), "not found");
                let owned: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM social_agents WHERE id=?1 AND owner_id=?2)",
                    params![agent, a],
                    |r| r.get(0),
                )?;
                anyhow::ensure!(
                    owned || room_owner(&tx, room, key)? == Some(true),
                    "not found"
                );
                tx.execute(
                    "DELETE FROM social_admissions WHERE room_id=?1 AND agent_id=?2",
                    params![room, agent],
                )?;
                json!({"ok":true})
            }
            ("DELETE", ["rooms", room, "membership"]) => {
                anyhow::ensure!(room_access(&tx, room, key)? == Some(true), "not found");
                anyhow::ensure!(
                    room_owner(&tx, room, key)? != Some(true),
                    "owner cannot leave"
                );
                tx.execute(
                    "DELETE FROM social_members WHERE room_id=?1 AND account_id=?2",
                    params![room, a],
                )?;
                tx.execute("DELETE FROM social_admissions WHERE room_id=?1 AND agent_id IN (SELECT id FROM social_agents WHERE owner_id=?2)",params![room,a])?;
                tx.execute("UPDATE social_room_invitations SET status='declined' WHERE room_id=?1 AND status='pending' AND (from_id=?2 OR to_id=?2)",params![room,a])?;
                json!({"ok":true})
            }
            _ => anyhow::bail!("not found"),
        };
        tx.commit()?;
        Ok(response)
    }
}
fn friend_request(conn: &Connection, id: &str) -> anyhow::Result<Value> {
    let (from, to, status, created): (String, String, String, String) = conn.query_row(
        "SELECT from_id,to_id,status,created_at FROM social_friend_requests WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(
        json!({"id":id,"from":account(conn,&from)?,"to":account(conn,&to)?,"status":status,"created_at":created}),
    )
}
pub fn credential(conn: &Connection, account: &str, agent: Option<&str>) -> anyhow::Result<String> {
    let token = format!(
        "{PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    conn.execute("INSERT INTO social_credentials(digest,account_id,agent_id,expires) VALUES(?1,?2,?3,unixepoch()+?4)",params![hash(&token),account,agent,if agent.is_some(){365*86400}else{30*86400}])?;
    Ok(token)
}
impl Store {
    pub fn social_oauth_start(
        &self,
        provider: &str,
        challenge: &str,
        key: Option<&str>,
        callback_port: u16,
        source: &str,
    ) -> anyhow::Result<(String, String)> {
        use base64::Engine;
        anyhow::ensure!(callback_port >= 1024, "invalid callback_port");
        anyhow::ensure!(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(challenge)?
                .len()
                == 32,
            "invalid code_challenge"
        );
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM social_oauth WHERE expires<unixepoch()", [])?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM social_oauth", [], |r| r.get(0))?;
        anyhow::ensure!(count < 1000, "too many sign-in attempts");
        let per_source: i64 = tx.query_row(
            "SELECT COUNT(*) FROM social_oauth WHERE source_hash=?1",
            [hash(source)],
            |r| r.get(0),
        )?;
        anyhow::ensure!(per_source < 20, "too many sign-in attempts");
        let link = if let Some(key) = key {
            let p = principal(&tx, key)?.ok_or_else(|| anyhow::anyhow!("unauthorized"))?;
            anyhow::ensure!(p.agent_id.is_none(), "human approval required");
            Some(p.account_id)
        } else {
            None
        };
        let request = uuid::Uuid::new_v4().to_string();
        let state = format!(
            "{request}.{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        tx.execute("INSERT INTO social_oauth(request_hash,state_hash,provider,challenge,link_account,link_digest,callback_port,source_hash,expires) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,unixepoch()+600)",params![hash(&request),hash(&state),provider,challenge,link,key.map(hash),callback_port,hash(source)])?;
        tx.commit()?;
        Ok((request, state))
    }
    pub fn social_oauth_claim(&self, provider: &str, state: &str) -> anyhow::Result<()> {
        let n=self.conn.lock().unwrap().execute("UPDATE social_oauth SET claimed=1 WHERE state_hash=?1 AND provider=?2 AND expires>=unixepoch() AND claimed=0 AND (link_digest IS NULL OR EXISTS(SELECT 1 FROM social_credentials c WHERE c.digest=social_oauth.link_digest AND c.revoked=0 AND c.expires>=unixepoch()))",params![hash(state),provider])?;
        anyhow::ensure!(n == 1, "invalid or expired sign-in");
        Ok(())
    }
    pub fn social_oauth_complete(
        &self,
        provider: &str,
        state: &str,
        subject: &str,
        label: &str,
    ) -> anyhow::Result<(u16, String)> {
        anyhow::ensure!(
            !subject.is_empty() && subject.len() <= 256,
            "invalid provider identity"
        );
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let port:u16=tx.query_row("SELECT callback_port FROM social_oauth WHERE state_hash=?1 AND provider=?2 AND expires>=unixepoch() AND claimed=1 AND completion_hash IS NULL AND (link_digest IS NULL OR EXISTS(SELECT 1 FROM social_credentials c WHERE c.digest=social_oauth.link_digest AND c.revoked=0 AND c.expires>=unixepoch()))",params![hash(state),provider],|r|r.get(0))?;
        let completion = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        tx.execute("UPDATE social_oauth SET provider_subject=?1,provider_label=?2,completion_hash=?3 WHERE state_hash=?4",params![subject,label.chars().filter(|c|!c.is_control()).take(100).collect::<String>(),hash(&completion),hash(state)])?;
        tx.commit()?;
        Ok((port, completion))
    }
    pub fn social_oauth_exchange(
        &self,
        request: &str,
        verifier: &str,
        completion: &str,
    ) -> anyhow::Result<Value> {
        use base64::Engine;
        anyhow::ensure!(
            (43..=128).contains(&verifier.len())
                && verifier
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)),
            "invalid code_verifier"
        );
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let (provider,subject,label,link):(String,String,String,Option<String>)=tx.query_row("SELECT provider,provider_subject,provider_label,link_account FROM social_oauth WHERE request_hash=?1 AND challenge=?2 AND completion_hash=?3 AND expires>=unixepoch() AND provider_subject IS NOT NULL AND (link_digest IS NULL OR EXISTS(SELECT 1 FROM social_credentials c WHERE c.digest=social_oauth.link_digest AND c.revoked=0 AND c.expires>=unixepoch()))",params![hash(request),challenge,hash(completion)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|anyhow::anyhow!("invalid or expired sign-in"))?;
        let provider = provider.as_str();
        let subject = subject.as_str();
        let label = label.as_str();
        let existing: Option<String> = tx
            .query_row(
                "SELECT account_id FROM social_identities WHERE provider=?1 AND subject=?2",
                params![provider, subject],
                |r| r.get(0),
            )
            .optional()?;
        let id = match (link, existing) {
            (Some(link), Some(existing)) => {
                anyhow::ensure!(link == existing, "identity already linked");
                link
            }
            (Some(link), None) => link,
            (None, Some(existing)) => existing,
            (None, None) => {
                let id = uuid::Uuid::new_v4().to_string();
                let handle = default_handle(&tx, label)?;
                tx.execute(
                    "INSERT INTO social_accounts(id,handle,display_name) VALUES(?1,?2,?3)",
                    params![
                        id,
                        handle,
                        label
                            .chars()
                            .filter(|c| display_char(*c))
                            .take(100)
                            .collect::<String>()
                    ],
                )?;
                id
            }
        };
        tx.execute("INSERT OR IGNORE INTO social_identities(provider,subject,account_id,label) VALUES(?1,?2,?3,?4)",params![provider,subject,id,label.chars().filter(|c|!c.is_control()).take(100).collect::<String>()])?;
        let key = credential(&tx, &id, None)?;
        tx.execute(
            "DELETE FROM social_oauth WHERE request_hash=?1",
            [hash(request)],
        )?;
        let response = json!({"status":"complete","device_key":key,"account":account(&tx,&id)?});
        tx.commit()?;
        Ok(response)
    }
}

fn bounded(conn: &Connection, sql: &str, account: &str, limit: i64) -> anyhow::Result<()> {
    let count: i64 = conn.query_row(sql, [account], |r| r.get(0))?;
    anyhow::ensure!(count < limit, "account limit reached");
    Ok(())
}
fn display_char(c: char) -> bool {
    !c.is_control()
        && !matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'|'\u{200e}'|'\u{200f}'|'\u{061c}')
}

#[cfg(test)]
impl Store {
    pub(crate) fn social_assert_hash_only(&self) {
        let conn = self.conn.lock().unwrap();
        for (table, column) in [
            ("social_credentials", "digest"),
            ("social_oauth", "request_hash"),
            ("agent_identities", "owner_key"),
            ("rooms", "owner_key"),
            ("room_grants", "api_key"),
        ] {
            let count: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE CAST({column} AS TEXT) LIKE '%csoc_%'"
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "raw credential in {table}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    fn login(store: &Store, subject: &str) -> (String, Value) {
        login_named(store, subject, subject)
    }
    fn login_named(store: &Store, subject: &str, name: &str) -> (String, Value) {
        let verifier = "v".repeat(64);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let (request, state) = store
            .social_oauth_start("github", &challenge, None, 49233, "test-source")
            .unwrap();
        store.social_oauth_claim("github", &state).unwrap();
        let (_, code) = store
            .social_oauth_complete("github", &state, subject, name)
            .unwrap();
        let response = store
            .social_oauth_exchange(&request, &verifier, &code)
            .unwrap();
        (
            response["device_key"].as_str().unwrap().to_owned(),
            response["account"].clone(),
        )
    }
    #[test]
    fn friend_alias_is_permanent_idempotent_and_requires_explicit_consent() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("test.db")).unwrap();
        let (alice, a) = login_named(&store, "alice", "Chad DePue");
        let (bob, b) = login(&store, "bob");
        let issue = || {
            store
                .social_action(&alice, "POST", "me/friend-link", &json!({}))
                .unwrap()
        };
        let link = issue();
        assert_eq!(issue(), link);
        let slug = link["slug"].as_str().unwrap();
        assert_eq!(slug, "Chad-DePue");
        store
            .social_action(
                &alice,
                "PATCH",
                "me",
                &json!({"handle":"Renamed","display_name":"New name"}),
            )
            .unwrap();
        store
            .social_action(
                &bob,
                "PATCH",
                "me",
                &json!({"handle":slug,"display_name":"Bob"}),
            )
            .unwrap();
        let bob_link = store
            .social_action(&bob, "POST", "me/friend-link", &json!({}))
            .unwrap();
        assert_eq!(bob_link["slug"], "Chad-DePue-0");
        let preview_path = format!("friend-links/{slug}");
        let preview = store
            .social_action(&bob, "GET", &preview_path, &json!({}))
            .unwrap();
        assert_eq!(preview["account"]["id"], a["id"]);
        assert_eq!(preview["account"]["handle"], "Renamed");
        assert_eq!(preview["relationship"], "none");
        assert_eq!(
            store
                .social_action(&bob, "GET", "friend-requests", &json!({}))
                .unwrap()["requests"],
            json!([])
        );
        let request_path = format!("{preview_path}/request");
        let sent = store
            .social_action(&bob, "POST", &request_path, &json!({}))
            .unwrap();
        assert_eq!(
            sent,
            store
                .social_action(&bob, "POST", &request_path, &json!({}))
                .unwrap()
        );
        assert_eq!(sent["request"]["from"]["id"], b["id"]);
        let incoming = store
            .social_action(
                &alice,
                "GET",
                &format!("friend-links/{}", bob_link["slug"].as_str().unwrap()),
                &json!({}),
            )
            .unwrap();
        assert_eq!(incoming["relationship"], "incoming_pending");
        assert!(incoming["request_id"].is_string());
        assert_eq!(
            store
                .social_action(
                    &alice,
                    "POST",
                    &format!(
                        "friend-links/{}/request",
                        bob_link["slug"].as_str().unwrap()
                    ),
                    &json!({})
                )
                .unwrap_err()
                .to_string(),
            "incoming_pending"
        );
        assert!(store
            .social_action(&alice, "POST", &request_path, &json!({}))
            .is_err());
        assert!(store.social_friend_link_account("unknown").is_err());
        let conn = store.conn.lock().unwrap();
        for table in ["social_friends", "social_members", "social_admissions"] {
            assert_eq!(
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        // Reservations intentionally survive deletion instead of retargeting links.
        conn.execute(
            "DELETE FROM social_accounts WHERE id=?1",
            [a["id"].as_str().unwrap()],
        )
        .unwrap();
        assert!(link_target(&conn, slug).is_err());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM social_friend_links WHERE slug=?1",
                [slug],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn availability_and_setup_handle_conflicts_are_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("test.db")).unwrap();
        let (alice, _) = login(&store, "alice");
        let (bob, b) = login(&store, "bob");
        assert_eq!(
            store
                .social_action(
                    &alice,
                    "POST",
                    "handles/availability",
                    &json!({"handle":"a"})
                )
                .unwrap()["valid"],
            false
        );
        assert_eq!(
            store
                .social_action(
                    &alice,
                    "POST",
                    "handles/availability",
                    &json!({"handle":"ALICE"})
                )
                .unwrap()["available"],
            true
        );
        assert_eq!(
            store
                .social_action(
                    &bob,
                    "POST",
                    "handles/availability",
                    &json!({"handle":"ALICE"})
                )
                .unwrap()["available"],
            false
        );
        let setup = store
            .social_action(
                &alice,
                "POST",
                "setup",
                &json!({"display_name":"First","handle":"reserved"}),
            )
            .unwrap();
        let failed = store
            .social_action(
                &bob,
                "POST",
                "setup",
                &json!({"display_name":"Changed","handle":"RESERVED"}),
            )
            .unwrap_err();
        assert_eq!(failed.to_string(), "handle_unavailable");
        assert_eq!(
            store.social_action(&bob, "GET", "me", &json!({})).unwrap()["account"],
            b
        );
        assert!(store
            .social_action(&bob, "GET", "setup", &json!({}))
            .unwrap()["setup"]
            .is_null());
        assert_eq!(
            store
                .social_action(
                    &bob,
                    "PATCH",
                    "me",
                    &json!({"display_name":"Changed","handle":"RESERVED"})
                )
                .unwrap_err()
                .to_string(),
            "handle_unavailable"
        );
        assert_eq!(
            store
                .social_action(
                    &alice,
                    "POST",
                    "setup",
                    &json!({"display_name":"Ignored","handle":"ignored"})
                )
                .unwrap(),
            setup
        );
        assert_eq!(
            store
                .social_action(&alice, "GET", "me", &json!({}))
                .unwrap()["account"]["handle"],
            "reserved"
        );
        let conn = store.conn.lock().unwrap();
        for table in ["social_agents", "social_rooms"] {
            assert_eq!(
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
    #[test]
    fn simultaneous_alias_issuance_and_handle_reservation_converge() {
        let directory = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(Store::open(&directory.path().join("test.db")).unwrap());
        let (alice, _) = login(&store, "alice");
        let (bob, _) = login(&store, "bob");
        let mut threads = Vec::new();
        for key in [alice.clone(), alice.clone(), bob.clone(), bob.clone()] {
            let store = store.clone();
            threads.push(std::thread::spawn(move || {
                store
                    .social_action(&key, "POST", "me/friend-link", &json!({}))
                    .unwrap()
            }));
        }
        let links = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(links[0], links[1]);
        assert_eq!(links[2], links[3]);
        assert_ne!(links[0], links[2]);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads = [alice, bob]
            .into_iter()
            .map(|key| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.social_action(
                        &key,
                        "POST",
                        "setup",
                        &json!({"display_name":"Winner","handle":"same-handle"}),
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| r
                    .as_ref()
                    .is_err_and(|e| e.to_string() == "handle_unavailable"))
                .count(),
            1
        );
        let conn = store.conn.lock().unwrap();
        for table in [
            "social_setup",
            "social_agents",
            "social_rooms",
            "social_members",
            "social_admissions",
        ] {
            assert_eq!(
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
    #[test]
    fn setup_is_durable_private_credentialless_and_never_repairs_revocation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("setup.db");
        let store = Store::open(&path).unwrap();
        let (key, owner) = login(&store, "owner");
        let (other, _) = login(&store, "other");
        assert_eq!(
            store
                .social_action(&key, "GET", "setup", &json!({}))
                .unwrap(),
            json!({"setup":null})
        );
        let initial = store
            .social_action(
                &key,
                "POST",
                "setup",
                &json!({"display_name":" Chad DePue "}),
            )
            .unwrap();
        let data = &initial["setup"];
        assert_eq!(data["agent_name"], "Chad_DePue_bot");
        assert_eq!(data["room_name"], "Chad_DePue_Home");
        assert_eq!(data["agent"]["discoverable"], false);
        assert_eq!(data["bot_admitted"], true);
        assert!(!initial.to_string().contains("api_key"));
        let room = data["room_id"].as_str().unwrap();
        let agent = data["agent_id"].as_str().unwrap();
        assert_eq!(store.social_room_access(room, &other), Some(false));
        assert_eq!(
            store
                .social_action(&other, "GET", "rooms", &json!({}))
                .unwrap()["rooms"],
            json!([])
        );
        assert_eq!(
            store.social_action(&key, "GET", "me", &json!({})).unwrap()["account"]["display_name"],
            "Chad DePue"
        );
        assert_eq!(
            store
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM social_credentials WHERE agent_id=?1",
                    [agent],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .social_action(&key, "POST", "setup", &json!({"display_name":null}))
                .unwrap(),
            initial
        );
        let endpoint = format!("agents/{agent}/credentials");
        assert!(store
            .social_action(&other, "POST", &endpoint, &json!({}))
            .is_err());
        let credential1 = store
            .social_action(&key, "POST", &endpoint, &json!({}))
            .unwrap();
        let credential2 = store
            .social_action(&key, "POST", &endpoint, &json!({}))
            .unwrap();
        let k1 = credential1["api_key"].as_str().unwrap();
        assert!(store.social_principal(k1).is_some());
        assert!(store
            .social_principal(credential2["api_key"].as_str().unwrap())
            .is_some());
        assert!(store.social_action(k1, "GET", "setup", &json!({})).is_err());
        assert!(store
            .social_action(k1, "POST", &endpoint, &json!({}))
            .is_err());
        store
            .social_action(
                &key,
                "DELETE",
                &format!("rooms/{room}/agents/{agent}"),
                &json!({}),
            )
            .unwrap();
        let no_admission = store
            .social_action(&key, "POST", "setup", &json!({}))
            .unwrap();
        assert_eq!(no_admission["setup"]["bot_admitted"], false);
        store
            .social_action(&key, "DELETE", &format!("agents/{agent}"), &json!({}))
            .unwrap();
        assert!(store
            .social_action(&key, "POST", &endpoint, &json!({}))
            .is_err());
        assert!(store.social_principal(k1).is_none());
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM social_members WHERE room_id=?1 AND account_id=?2",
                params![room, owner["id"].as_str().unwrap()],
            )
            .unwrap();
        let unavailable = store
            .social_action(&key, "POST", "setup", &json!({}))
            .unwrap();
        assert!(unavailable["setup"]["agent"].is_null());
        assert!(unavailable["setup"]["room"].is_null());
        store
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM rooms WHERE room_id=?1", [room])
            .unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened
                .social_action(&key, "POST", "setup", &json!({}))
                .unwrap(),
            unavailable
        );
    }
    #[test]
    fn connect_limits_active_credentials_without_rotating_or_reviving_agents() {
        let store = Store::open_in_memory().unwrap();
        let (key, _) = login(&store, "owner");
        let initial = store
            .social_action(&key, "POST", "setup", &json!({"display_name":"Owner"}))
            .unwrap();
        let id = initial["setup"]["agent_id"].as_str().unwrap();
        let endpoint = format!("agents/{id}/credentials");
        let mut keys = Vec::new();
        for _ in 0..20 {
            let issued = store
                .social_action(&key, "POST", &endpoint, &json!({}))
                .unwrap();
            keys.push(issued["api_key"].as_str().unwrap().to_owned());
        }
        assert_eq!(
            store
                .social_action(&key, "POST", &endpoint, &json!({}))
                .unwrap_err()
                .to_string(),
            "credential limit reached"
        );
        assert!(keys.iter().all(|key| store.social_principal(key).is_some()));
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE social_credentials SET expires=unixepoch()-1 WHERE digest=?1",
                [hash(&keys[0])],
            )
            .unwrap();
        let replacement = store
            .social_action(&key, "POST", &endpoint, &json!({}))
            .unwrap();
        assert!(store
            .social_principal(replacement["api_key"].as_str().unwrap())
            .is_some());
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE social_credentials SET revoked=1 WHERE digest=?1",
                [hash(&keys[1])],
            )
            .unwrap();
        let replacement2 = store
            .social_action(&key, "POST", &endpoint, &json!({}))
            .unwrap();
        assert!(store
            .social_principal(replacement2["api_key"].as_str().unwrap())
            .is_some());
        assert!(keys[2..]
            .iter()
            .all(|key| store.social_principal(key).is_some()));
        store
            .social_action(&key, "DELETE", &format!("agents/{id}"), &json!({}))
            .unwrap();
        assert_eq!(
            store
                .social_action(&key, "POST", &endpoint, &json!({}))
                .unwrap_err()
                .to_string(),
            "not found"
        );
        assert!(keys.iter().all(|key| store.social_principal(key).is_none()));
        assert!(store
            .social_action(&key, "POST", "setup", &json!({}))
            .unwrap()["setup"]["agent"]
            .is_null());
    }
    #[test]
    fn concurrent_setup_and_failed_commit_do_not_duplicate_or_leave_partial_resources() {
        let store = std::sync::Arc::new(Store::open_in_memory().unwrap());
        let (key, _) = login(&store, "concurrent");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let store = store.clone();
                let key = key.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .social_action(
                            &key,
                            "POST",
                            "setup",
                            &json!({"display_name":"Concurrent User"}),
                        )
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(results.iter().all(|result| result == &results[0]));
        assert_eq!(
            store
                .social_action(&key, "GET", "agents", &json!({}))
                .unwrap()["agents"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .social_action(&key, "GET", "rooms", &json!({}))
                .unwrap()["rooms"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let (failed, failed_account) = login(&store, "failed");
        store.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_setup BEFORE INSERT ON social_setup BEGIN SELECT RAISE(ABORT,'injected setup failure'); END;").unwrap();
        assert!(store
            .social_action(&failed, "POST", "setup", &json!({"display_name":"Changed"}))
            .is_err());
        assert_eq!(
            store
                .social_action(&failed, "GET", "setup", &json!({}))
                .unwrap(),
            json!({"setup":null})
        );
        assert_eq!(
            store
                .social_action(&failed, "GET", "agents", &json!({}))
                .unwrap()["agents"],
            json!([])
        );
        assert_eq!(
            store
                .social_action(&failed, "GET", "rooms", &json!({}))
                .unwrap()["rooms"],
            json!([])
        );
        assert_eq!(
            store
                .social_action(&failed, "GET", "me", &json!({}))
                .unwrap()["account"],
            failed_account
        );
        let conn = store.conn.lock().unwrap();
        for table in [
            "rooms",
            "social_agents",
            "social_rooms",
            "social_members",
            "social_admissions",
            "social_setup",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                count,
                if table == "rooms" { 2 } else { 1 },
                "partial write in {table}"
            );
        }
    }
    #[test]
    fn setup_collision_quota_rollback_and_default_names() {
        let store = Store::open_in_memory().unwrap();
        let (key, account) = login(&store, "one");
        let (other, _) = login(&store, "two");
        store
            .social_action(&other, "POST", "rooms", &json!({"name":"Chad_DePue_Home"}))
            .unwrap();
        let created = store
            .social_action(&key, "POST", "setup", &json!({"display_name":"Chad DePue"}))
            .unwrap();
        assert_eq!(created["setup"]["room_name"], "Chad_DePue_Home_0");
        let default = store
            .social_action(&key, "POST", "agents", &json!({}))
            .unwrap();
        assert_eq!(default["agent"]["name"], "Chad_DePue_bot");
        assert_eq!(starter_stem(" /🦄 "), "Cowboy");
        assert_eq!(starter_stem("Éléonore 王"), "Éléonore_王");
        let other_id = store
            .social_action(&other, "GET", "me", &json!({}))
            .unwrap()["account"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        for index in 0..100 {
            store.conn.lock().unwrap().execute("INSERT INTO social_agents(id,owner_id,name,discoverable) VALUES(?1,?2,'full',0)",params![format!("full-{index}"),other_id]).unwrap();
        }
        assert!(store
            .social_action(
                &other,
                "POST",
                "setup",
                &json!({"display_name":"Should roll back"})
            )
            .is_err());
        assert_eq!(
            store
                .social_action(&other, "GET", "setup", &json!({}))
                .unwrap(),
            json!({"setup":null})
        );
        assert_eq!(
            store
                .social_action(&other, "GET", "me", &json!({}))
                .unwrap()["account"]["display_name"],
            "two"
        );
        let conn = store.conn.lock().unwrap();
        conn.execute("DELETE FROM social_agents WHERE owner_id=?1", [&other_id])
            .unwrap();
        for index in 0..100 {
            conn.execute(
                "INSERT INTO social_rooms(room_id,owner_id) VALUES(?1,?2)",
                params![format!("quota-{index}"), other_id],
            )
            .unwrap();
        }
        drop(conn);
        assert!(store
            .social_action(
                &other,
                "POST",
                "setup",
                &json!({"display_name":"Should roll back"})
            )
            .is_err());
        assert_eq!(
            store
                .social_action(&other, "GET", "agents", &json!({}))
                .unwrap()["agents"],
            json!([])
        );
        assert_eq!(
            store.social_action(&key, "GET", "me", &json!({})).unwrap()["account"]["id"],
            account["id"]
        );
    }
    #[test]
    fn solo_room_admits_only_creator_and_approved_owned_agents() {
        let store = Store::open_in_memory().unwrap();
        let (key, owner) = login(&store, "owner");
        let (other_key, _) = login(&store, "other");
        let approved = store
            .social_action(&key, "POST", "agents", &json!({"name":"approved"}))
            .unwrap();
        let unapproved = store
            .social_action(&key, "POST", "agents", &json!({"name":"unapproved"}))
            .unwrap();
        let foreign = store
            .social_action(&other_key, "POST", "agents", &json!({"name":"foreign"}))
            .unwrap();
        let response = store
            .social_action(
                &key,
                "POST",
                "rooms",
                &json!({"name":"solo","agent_ids":[approved["agent"]["id"]]}),
            )
            .unwrap();
        assert!(response["invitation"].is_null());
        let room = response["room_id"].as_str().unwrap();
        assert_eq!(store.social_room_owner(room, &key), Some(true));
        assert_eq!(store.social_room_owner(room, &other_key), Some(false));
        for (credential, allowed) in [
            (key.as_str(), true),
            (approved["api_key"].as_str().unwrap(), true),
            (unapproved["api_key"].as_str().unwrap(), false),
            (other_key.as_str(), false),
            (foreign["api_key"].as_str().unwrap(), false),
        ] {
            assert_eq!(store.social_room_access(room, credential), Some(allowed));
        }
        let participants = store
            .social_action(
                &key,
                "GET",
                &format!("rooms/{room}/participants"),
                &json!({}),
            )
            .unwrap();
        assert_eq!(participants["members"], json!([owner]));
        assert_eq!(participants["agents"], json!([approved["agent"]]));
        assert_eq!(
            store
                .social_action(&key, "GET", "room-invitations", &json!({}))
                .unwrap(),
            json!({"invitations":[]})
        );
        let conn = store.conn.lock().unwrap();
        let (visibility, owner_key, encrypted): (String, String, bool) = conn
            .query_row(
                "SELECT visibility,owner_key,encrypted FROM rooms WHERE room_id=?1",
                [room],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(visibility, "private");
        assert_eq!(
            owner_key,
            format!("{OWNER_PREFIX}{}", owner["id"].as_str().unwrap())
        );
        assert!(!encrypted);
        drop(conn);
        assert!(store
            .social_action(
                approved["api_key"].as_str().unwrap(),
                "POST",
                "rooms",
                &json!({"name":"agent-created"})
            )
            .is_err());
    }
    #[test]
    fn invalid_room_creation_rolls_back_every_record() {
        let store = Store::open_in_memory().unwrap();
        let (key, _) = login(&store, "owner");
        let (other_key, _) = login(&store, "other");
        let owned = store
            .social_action(&key, "POST", "agents", &json!({"name":"owned"}))
            .unwrap();
        let foreign = store
            .social_action(&other_key, "POST", "agents", &json!({"name":"foreign"}))
            .unwrap();
        let before: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM rooms", [], |row| row.get(0))
            .unwrap();
        let invalid_fields = [
            json!({"friend_id":null}),
            json!({"friend_id":""}),
            json!({"friend_id":"  "}),
            json!({"friend_id":42}),
            json!({"friend_id":false}),
            json!({"friend_id":[]}),
            json!({"friend_id":{}}),
            json!({"friend_id":"not-a-friend"}),
            json!({"proposed_agent_ids":null}),
            json!({"proposed_agent_ids":"bad"}),
            json!({"proposed_agent_ids":[42]}),
            json!({"proposed_agent_ids":["someone"]}),
            json!({"agent_ids":[owned["agent"]["id"],foreign["agent"]["id"]]}),
        ];
        for mut body in invalid_fields {
            body["name"] = json!("invalid");
            assert!(
                store.social_action(&key, "POST", "rooms", &body).is_err(),
                "accepted {body}"
            );
            let conn = store.conn.lock().unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM rooms", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, before, "room leaked for {body}");
            for table in [
                "social_rooms",
                "social_members",
                "social_admissions",
                "social_room_invitations",
            ] {
                let count: i64 = conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, 0, "{table} leaked for {body}");
            }
        }
    }
    #[test]
    fn room_with_friend_keeps_pending_invitation_and_admission_approval() {
        let store = Store::open_in_memory().unwrap();
        let (key, _) = login(&store, "owner");
        let (friend_key, friend) = login(&store, "friend");
        let request = store
            .social_action(
                &key,
                "POST",
                "friend-requests",
                &json!({"handle":friend["handle"]}),
            )
            .unwrap();
        store
            .social_action(
                &friend_key,
                "POST",
                &format!(
                    "friend-requests/{}/accept",
                    request["request"]["id"].as_str().unwrap()
                ),
                &json!({}),
            )
            .unwrap();
        let agent = store
            .social_action(
                &friend_key,
                "POST",
                "agents",
                &json!({"name":"friend agent"}),
            )
            .unwrap();
        let room = store.social_action(&key, "POST", "rooms", &json!({"name":"shared","friend_id":friend["id"],"proposed_agent_ids":[agent["agent"]["id"]]})).unwrap();
        let room_id = room["room_id"].as_str().unwrap();
        assert_eq!(room["invitation"]["status"], "pending");
        assert_eq!(room["invitation"]["to"], friend);
        assert_eq!(
            room["invitation"]["proposed_agents"],
            json!([agent["agent"]])
        );
        assert_eq!(store.social_room_access(room_id, &friend_key), Some(false));
        assert_eq!(
            store.social_room_access(room_id, agent["api_key"].as_str().unwrap()),
            Some(false)
        );
        store
            .social_action(
                &friend_key,
                "POST",
                &format!(
                    "room-invitations/{}/accept",
                    room["invitation"]["id"].as_str().unwrap()
                ),
                &json!({"agent_ids":[agent["agent"]["id"]]}),
            )
            .unwrap();
        assert_eq!(store.social_room_access(room_id, &friend_key), Some(true));
        assert_eq!(
            store.social_room_access(room_id, agent["api_key"].as_str().unwrap()),
            Some(true)
        );
    }
    #[test]
    fn name_based_handles_preserve_case_and_disambiguate_from_zero() {
        let store = Store::open_in_memory().unwrap();
        let (key, first) = login_named(&store, "first", "Chad DePue");
        assert_eq!(first["handle"], "Chad-DePue");
        let (_, second) = login_named(&store, "second", "Chad DePue");
        assert_eq!(second["handle"], "Chad-DePue-0");
        let (_, third) = login_named(&store, "third", "Chad DePue");
        assert_eq!(third["handle"], "Chad-DePue-1");
        let (_, fourth) = login_named(&store, "fourth", "CHAD DEPUE");
        assert_eq!(fourth["handle"], "CHAD-DEPUE-2");
        let (_, returning) = login_named(&store, "first", "Changed Name");
        assert_eq!(returning["id"], first["id"]);
        assert_eq!(returning["handle"], "Chad-DePue");
        let updated = store
            .social_action(
                &key,
                "PATCH",
                "me",
                &json!({"handle":"Chad-DePue","display_name":"Chad DePue"}),
            )
            .unwrap();
        assert_eq!(updated["account"]["handle"], "Chad-DePue");
        let (other_key, _) = login_named(&store, "other", "Other Person");
        let request = store
            .social_action(
                &other_key,
                "POST",
                "friend-requests",
                &json!({"handle":"cHaD-dEpUe"}),
            )
            .unwrap();
        assert_eq!(request["request"]["to"]["id"], first["id"]);
        assert!(store
            .social_action(
                &other_key,
                "PATCH",
                "me",
                &json!({"handle":"chad-depue","display_name":"Other Person"})
            )
            .is_err());
    }
    #[test]
    fn default_handles_are_bounded_and_do_not_replace_chosen_handles() {
        let store = Store::open_in_memory().unwrap();
        let (_, first) = login_named(&store, "long-first", &"A".repeat(100));
        let (_, second) = login_named(&store, "long-second", &"A".repeat(100));
        assert_eq!(first["handle"], "A".repeat(40));
        assert_eq!(second["handle"], format!("{}-0", "A".repeat(38)));
        let (_, fallback) = login_named(&store, "fallback", "🐄");
        assert_eq!(fallback["handle"], "Cowboy");
        let (key, _) = login_named(&store, "custom", "Some Name");
        store
            .social_action(
                &key,
                "PATCH",
                "me",
                &json!({"handle":"My-Chosen-Handle","display_name":"Some Name"}),
            )
            .unwrap();
        let (_, returning) = login_named(&store, "custom", "Different Name");
        assert_eq!(returning["handle"], "My-Chosen-Handle");
    }
    #[test]
    fn restart_preserves_admissions_expiry_and_atomic_friend_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("social.sqlite");
        let (ak, bk, agent_key, agent_id, rid);
        {
            let store = Store::open(&db).unwrap();
            let (a, alice) = login(&store, "alice");
            let (b, bob) = login(&store, "bob");
            ak = a;
            bk = b;
            let request = store
                .social_action(
                    &ak,
                    "POST",
                    "friend-requests",
                    &json!({"handle":bob["handle"]}),
                )
                .unwrap();
            let request = request["request"]["id"].as_str().unwrap();
            store
                .social_action(
                    &bk,
                    "POST",
                    &format!("friend-requests/{request}/accept"),
                    &json!({}),
                )
                .unwrap();
            let agent = store
                .social_action(&ak, "POST", "agents", &json!({"name":"stable agent"}))
                .unwrap();
            agent_key = agent["api_key"].as_str().unwrap().to_owned();
            agent_id = agent["agent"]["id"].as_str().unwrap().to_owned();
            let room = store
                .social_action(
                    &ak,
                    "POST",
                    "rooms",
                    &json!({"name":"durable","friend_id":bob["id"],"agent_ids":[agent_id]}),
                )
                .unwrap();
            rid = room["room_id"].as_str().unwrap().to_owned();
            assert!(store.claim_agent_identity(&agent_id, &agent_key).unwrap());
            let invitation = room["invitation"]["id"].as_str().unwrap();
            store
                .social_action(
                    &ak,
                    "DELETE",
                    &format!("friends/{}", bob["id"].as_str().unwrap()),
                    &json!({}),
                )
                .unwrap();
            assert!(store
                .social_action(
                    &bk,
                    "POST",
                    &format!("room-invitations/{invitation}/accept"),
                    &json!({"agent_ids":[]})
                )
                .is_err());
            assert_ne!(alice["id"], bob["id"]);
            store.social_assert_hash_only();
        }
        let store = Store::open(&db).unwrap();
        assert_eq!(store.social_room_access(&rid, &agent_key), Some(true));
        assert_eq!(store.social_room_access(&rid, &bk), Some(false));
        assert!(store.claim_agent_identity(&agent_id, &agent_key).unwrap());
        assert!(!store.social_identity_allowed(&agent_id, &ak));
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE social_credentials SET expires=unixepoch()-1 WHERE digest=?1",
                [hash(&agent_key)],
            )
            .unwrap();
        assert!(store.social_principal(&agent_key).is_none());
        assert_eq!(store.social_room_access(&rid, &agent_key), Some(false));
        let verifier = "q".repeat(64);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let (_, state) = store
            .social_oauth_start("github", &challenge, Some(&ak), 49233, "test-source")
            .unwrap();
        store
            .social_action(&ak, "POST", "logout", &json!({}))
            .unwrap();
        assert!(store.social_oauth_claim("github", &state).is_err());
    }
}
