use super::*;

const SUB: &str = "30000000-0000-4000-8000-000000000099";
const CANARY: &str = "private-diagnostics-canary";

fn target(room: &str, generation: u64) -> String {
    format!("/rooms/{room}/diagnostics?transport_generation={generation}")
}

fn get(
    http: &reqwest::Client,
    addr: std::net::SocketAddr,
    target: &str,
    nonce: u8,
    now: i64,
) -> reqwest::RequestBuilder {
    let (p, s) = signed_for("GET", target, b"", nonce, now);
    http.get(format!("http://{addr}{target}"))
        .header("x-cowchat-certificate", "fixture-cert")
        .header("x-cowchat-request", B64.encode(p))
        .header("x-cowchat-signature", B64.encode(s))
}

fn subscribe(store: &Store, now: i64) {
    store
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM subscriptions WHERE subscription_id='wake'", [])
        .unwrap();
    let body = serde_json::to_vec(&serde_json::json!({
        "subscription_id":SUB,"transport_generation":0,
        "webhook_url":format!("https://example.invalid/{CANARY}"),
        "secret":format!("{CANARY}-secret-secret"),"after":0
    }))
    .unwrap();
    let path = format!("/rooms/{ROOM}/subscriptions");
    let (p, s) = signed_for("POST", &path, &body, 1, now);
    store
        .subscribe_seated(ROOM, "fixture-cert", "POST", &path, &body, &p, &s, now)
        .unwrap();
}

#[tokio::test]
async fn diagnostics_http_allowlist_is_own_seat_only_and_never_claims_unsupported_health() {
    let state = crate::web::tests::test_state();
    let store = state.store.clone();
    install_fixture(&store);
    let now = Utc::now().timestamp_millis();
    let path = target(ROOM, 0);
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let http = reqwest::Client::new();
    let absent = get(&http, addr, &path, 10, now).send().await.unwrap();
    assert_eq!(absent.status(), 200);
    assert_eq!(absent.headers()["cache-control"], "no-store");
    let absent: serde_json::Value = absent.json().await.unwrap();
    assert!(absent["subscription"].is_null());
    subscribe(&store, now);
    let sealed = sealed(CANARY.as_bytes());
    append(&store, &sealed, 2, now).unwrap();
    // Public fixture state creates another seat's backlog. The endpoint must
    // neither count it nor serialize any stored error/status/secret strings.
    {
        let conn = store.conn.lock().unwrap();
        conn.execute("UPDATE subscription_deliveries SET last_error=?1", [CANARY])
            .unwrap();
        conn.execute("INSERT INTO subscriptions(subscription_id,room_id,owner_key,webhook_url,secret)
            SELECT 'other-sub',room_id,owner_key,webhook_url,secret FROM subscriptions WHERE subscription_id=?1", [SUB]).unwrap();
        conn.execute("INSERT INTO seated_subscriptions
            SELECT 'other-sub',room_id,cert_id,'other-seat',auth_generation,transport_generation,request_digest
            FROM seated_subscriptions WHERE subscription_id=?1", [SUB]).unwrap();
        conn.execute("INSERT INTO subscription_deliveries(delivery_id,subscription_id,message_seq,message_id,next_attempt_at)
            SELECT 'other-delivery','other-sub',message_seq,message_id,next_attempt_at FROM subscription_deliveries WHERE subscription_id=?1", [SUB]).unwrap();
        conn.execute(
            "INSERT INTO seated_wakes(delivery_id,payload) VALUES ('other-delivery',?1)",
            [CANARY],
        )
        .unwrap();
    }
    let response = get(&http, addr, &path, 11, now).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let bytes = response.text().await.unwrap();
    assert!(!bytes.contains(CANARY));
    assert!(!bytes.contains("other-seat"));
    assert!(!bytes.contains("other-sub"));
    let snapshot: serde_json::Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(
        snapshot,
        serde_json::json!({
            "version":1,"transport":{"kind":"sqlite","generation":0,"local_tip":1},
            "authorization_generation":1,"key_generation":2,
            "subscription":{"state":"active","binding_current":true,"pending_wakes":1},
            "checks":{"archive":"unsupported","production_runtime":"unsupported"}
        })
    );
    // Diagnostics observes stored states; it neither repairs nor claims a
    // currently bound subscription merely because the base row says active.
    for (index, status) in ["failed", "disabled", CANARY].iter().enumerate() {
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE subscriptions SET status=?1 WHERE subscription_id=?2",
                params![status, SUB],
            )
            .unwrap();
        let result = get(&http, addr, &path, 12 + index as u8, now)
            .send()
            .await
            .unwrap();
        let text = result.text().await.unwrap();
        assert!(!text.contains(CANARY));
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["subscription"]["state"],
            if *status == CANARY { "unknown" } else { status }
        );
        assert_eq!(value["subscription"]["pending_wakes"], 1);
    }
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE seated_subscriptions SET auth_generation=0 WHERE subscription_id=?1",
            [SUB],
        )
        .unwrap();
    let value: serde_json::Value = get(&http, addr, &path, 15, now)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(value["subscription"]["binding_current"], false);
    server.abort();
}

#[tokio::test]
async fn diagnostics_http_authentication_replay_and_query_binding_fail_closed() {
    let state = crate::web::tests::test_state();
    let store = state.store.clone();
    install_fixture(&store);
    let now = Utc::now().timestamp_millis();
    let path = target(ROOM, 0);
    let (server, addr) = crate::web::tests::start_test_web_server(state).await;
    let http = reqwest::Client::new();
    let response = http
        .get(format!("http://{addr}{path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(response.text().await.unwrap().is_empty());
    let mut forged = get(&http, addr, &path, 20, now).build().unwrap();
    forged
        .headers_mut()
        .insert("x-cowchat-signature", B64.encode([0; 64]).parse().unwrap());
    let response = http.execute(forged).await.unwrap();
    assert_eq!(response.status(), 401);
    assert!(response.text().await.unwrap().is_empty());
    assert_eq!(
        get(&http, addr, &target(ID, 0), 21, now)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        get(&http, addr, &path, 22, now - 600_000)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        get(&http, addr, &target(ROOM, 1), 23, now)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let (p, s) = signed_for("GET", TARGET, b"", 24, now);
    assert_eq!(
        http.get(format!("http://{addr}{path}"))
            .header("x-cowchat-certificate", "fixture-cert")
            .header("x-cowchat-request", B64.encode(p))
            .header("x-cowchat-signature", B64.encode(s))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        get(&http, addr, &path, 25, now)
            .body(CANARY)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let malformed = get(&http, addr, &format!("{path}&{CANARY}=1"), 26, now)
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 400);
    assert!(malformed.text().await.unwrap().is_empty());
    assert_eq!(
        get(&http, addr, &path, 27, now)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        get(&http, addr, &path, 27, now)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    // Current room authority is rechecked on every diagnostic read.
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation=2", [])
        .unwrap();
    assert_eq!(
        get(&http, addr, &path, 28, now)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE seated_rooms SET auth_generation=1", [])
        .unwrap();
    // Expire the already-authenticated fixture credential, independent of the
    // freshness of the request signature itself.
    {
        let conn = store.conn.lock().unwrap();
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT trusted_context FROM seated_credentials WHERE cert_id='fixture-cert'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let Value::Map(mut fields) = ciborium::from_reader(bytes.as_slice()).unwrap() else {
            panic!()
        };
        for (key, value) in &mut fields {
            if *key == text("expires_at") {
                *value = (now as u64 - 1).into();
            }
        }
        conn.execute(
            "UPDATE seated_credentials SET trusted_context=?1 WHERE cert_id='fixture-cert'",
            [encode(&Value::Map(fields))],
        )
        .unwrap();
    }
    assert_eq!(
        get(&http, addr, &path, 29, now)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    server.abort();
}
