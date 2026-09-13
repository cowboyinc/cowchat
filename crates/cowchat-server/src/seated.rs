//! Signed v3 append transport. Credential provisioning is a separate trusted
//! path; this endpoint never accepts membership or controller claims from a caller.
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use cowchat_crypto::canonical;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordHeader {
    pub v: u64,
    pub message_id: String,
    pub chain_id: u64,
    pub room: String,
    pub seat: String,
    pub role: String,
    pub via: Option<String>,
    pub via_sender: Option<String>,
    pub class: String,
    pub reply_to: Option<String>,
    pub mentions: Vec<String>,
    pub wake_hint: String,
    pub gen: u64,
    pub cert: String,
    pub nonce: String,
}

pub(crate) struct SealedRecord {
    pub header: RecordHeader,
    pub header_cbor: Vec<u8>,
    pub body: String,
    pub signature: Vec<u8>,
}
impl SealedRecord {
    pub fn parse(raw: &[u8]) -> Result<Self, crate::store::StoreError> {
        use crate::store::StoreError::SeatedAuthorization as Invalid;
        // Also bounded by the HTTP framework. Keep this bound for internal callers.
        if raw.len() > 2 * 1024 * 1024 {
            return Err(Invalid);
        }
        let mut record: serde_json::Value = serde_json::from_slice(raw).map_err(|_| Invalid)?;
        let fields = record.as_object_mut().ok_or(Invalid)?;
        let body = fields
            .remove("body")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(Invalid)?;
        let signature = fields
            .remove("sig")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(Invalid)?;
        let signature = B64.decode(signature).map_err(|_| Invalid)?;
        let header: RecordHeader = serde_json::from_value(record).map_err(|_| Invalid)?;
        uuid::Uuid::parse_str(&header.message_id).map_err(|_| Invalid)?;
        uuid::Uuid::parse_str(&header.room).map_err(|_| Invalid)?;
        if let Some(reply) = &header.reply_to {
            uuid::Uuid::parse_str(reply).map_err(|_| Invalid)?;
        }
        let mut header_cbor = Vec::new();
        ciborium::into_writer(&header, &mut header_cbor).map_err(|_| Invalid)?;
        let header_cbor = canonical::canonicalize(&header_cbor).map_err(|_| Invalid)?;
        Ok(Self {
            header,
            header_cbor,
            body,
            signature,
        })
    }
}

pub(crate) async fn append(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let decode_header = |name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| B64.decode(value).ok())
    };
    let (Some(projection), Some(signature)) = (
        decode_header("x-cowchat-request"),
        decode_header("x-cowchat-signature"),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let now = chrono::Utc::now().timestamp_millis();
    let target = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or(uri.path());
    match state.store.append_seated_record(
        &room,
        method.as_str(),
        target,
        &body,
        &projection,
        &signature,
        now,
    ) {
        Ok(result) => {
            state.webhook_mgr.wake();
            (
                StatusCode::OK,
                Json(serde_json::json!({"message_id": result.message.message_id,
                "status":"accepted", "seq":result.message.seq})),
            )
                .into_response()
        }
        Err(crate::store::StoreError::MessageConflict) => StatusCode::CONFLICT.into_response(),
        Err(crate::store::StoreError::SeatedReplay) => StatusCode::CONFLICT.into_response(),
        Err(
            crate::store::StoreError::SeatedAuthorization
            | crate::store::StoreError::SeatedRequestRequired,
        ) => StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerEnrollment {
    pub identity: String,
    pub identity_signature: String,
    pub membership: String,
    pub membership_signature: String,
}

pub(crate) async fn enroll_owner(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let Some(key) = crate::web::authenticated_key(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match state
        .store
        .enroll_seated_owner(&room, &key, &body, chrono::Utc::now().timestamp_millis())
    {
        Ok((seat, cert)) => (
            StatusCode::OK,
            Json(serde_json::json!({"room_id":room,"seat":seat,"cert":cert,"mode":"seated"})),
        )
            .into_response(),
        Err(crate::store::StoreError::MessageConflict) => StatusCode::CONFLICT.into_response(),
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn enroll_actor(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let Some(authority) = state.actor_proof_authority.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let decode = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| B64.decode(v).ok())
    };
    let (Some(projection), Some(signature)) =
        (decode("x-cowchat-request"), decode("x-cowchat-signature"))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    let prepared = match state.store.prepare_actor_enrollment(
        &room,
        target,
        &body,
        &projection,
        &signature,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(prepared) => prepared,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let proof = match authority.fetch(prepared.actor()).await {
        Ok(proof) => proof,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match state.store.install_actor_enrollment(
        prepared,
        proof,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok((seat, cert)) => (
            StatusCode::OK,
            Json(serde_json::json!({"room_id":room,"seat":seat,"cert":cert,"mode":"seated"})),
        )
            .into_response(),
        Err(crate::store::StoreError::MessageConflict | crate::store::StoreError::SeatedReplay) => {
            StatusCode::CONFLICT.into_response()
        }
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryQuery {
    pub transport_generation: u64,
    #[serde(default)]
    pub after: i64,
    #[serde(default = "default_page_limit")]
    pub limit: u32,
    pub message_id: Option<String>,
}
fn default_page_limit() -> u32 {
    100
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticsQuery {
    pub transport_generation: u64,
}

pub(crate) async fn diagnostics(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    query: Result<axum::extract::Query<DiagnosticsQuery>, axum::extract::rejection::QueryRejection>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let Ok(axum::extract::Query(query)) = query else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !body.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let decode_header = |name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| B64.decode(value).ok())
    };
    let (Some(cert), Some(projection), Some(signature)) = (
        headers
            .get("x-cowchat-certificate")
            .and_then(|value| value.to_str().ok()),
        decode_header("x-cowchat-request"),
        decode_header("x-cowchat-signature"),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let target = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or(uri.path());
    match state.store.read_seated_diagnostics(
        &room,
        cert,
        method.as_str(),
        target,
        &projection,
        &signature,
        query.transport_generation,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(snapshot) => (
            StatusCode::OK,
            [("cache-control", "no-store")],
            Json(snapshot),
        )
            .into_response(),
        Err(crate::store::StoreError::SeatedReplay) => StatusCode::CONFLICT.into_response(),
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MentionSubscription {
    pub subscription_id: String,
    pub transport_generation: u64,
    pub webhook_url: String,
    pub secret: String,
    pub after: Option<i64>,
}
impl MentionSubscription {
    pub(crate) fn validate(&self) -> Result<(), ()> {
        if uuid::Uuid::parse_str(&self.subscription_id).is_err()
            || self.after.is_some_and(|value| value < 0)
            || !(32..=512).contains(&self.secret.len())
            || self.webhook_url.len() > 2048
        {
            return Err(());
        }
        Ok(())
    }
}

pub(crate) async fn subscribe(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if body.len() > 16 * 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let Some(cert) = headers
        .get("x-cowchat-certificate")
        .and_then(|v| v.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let decode = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| B64.decode(v).ok())
    };
    let (Some(projection), Some(signature)) =
        (decode("x-cowchat-request"), decode("x-cowchat-signature"))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(input) = serde_json::from_slice::<MentionSubscription>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    if state
        .store
        .preflight_seated_subscription(
            &room,
            cert,
            method.as_str(),
            target,
            &body,
            &projection,
            &signature,
            chrono::Utc::now().timestamp_millis(),
        )
        .is_err()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.validate().is_err()
        || state
            .webhook_mgr
            .validate_url(&input.webhook_url)
            .await
            .is_err()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    match state.store.subscribe_seated(
        &room,
        cert,
        method.as_str(),
        target,
        &body,
        &projection,
        &signature,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(result) => {
            state.webhook_mgr.wake();
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(crate::store::StoreError::MessageConflict | crate::store::StoreError::SeatedReplay) => {
            StatusCode::CONFLICT.into_response()
        }
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubscriptionMutation {
    pub operation_id: String,
    pub transport_generation: u64,
    pub expected_revision: i64,
    pub action: SubscriptionAction,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SubscriptionAction {
    Update { webhook_url: String, secret: String },
    Repair,
    Delete,
}
impl SubscriptionMutation {
    pub(crate) fn validate(&self) -> Result<(), ()> {
        if uuid::Uuid::parse_str(&self.operation_id)
            .map(|id| id.to_string() != self.operation_id)
            .unwrap_or(true)
            || self.expected_revision < 0
            || self.expected_revision == i64::MAX
        {
            return Err(());
        }
        if let SubscriptionAction::Update {
            webhook_url,
            secret,
        } = &self.action
        {
            if webhook_url.len() > 2048 || !(32..=512).contains(&secret.len()) {
                return Err(());
            }
        }
        Ok(())
    }
}

pub(crate) async fn subscription_lifecycle(
    State(state): State<crate::web::AppState>,
    Path((room, subscription)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if body.len() > 16 * 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let Some(cert) = headers
        .get("x-cowchat-certificate")
        .and_then(|v| v.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let decode = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| B64.decode(v).ok())
    };
    let (Some(projection), Some(signature)) =
        (decode("x-cowchat-request"), decode("x-cowchat-signature"))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(input) = serde_json::from_slice::<SubscriptionMutation>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    if state
        .store
        .preflight_seated_subscription(
            &room,
            cert,
            method.as_str(),
            target,
            &body,
            &projection,
            &signature,
            chrono::Utc::now().timestamp_millis(),
        )
        .is_err()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.validate().is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let SubscriptionAction::Update { webhook_url, .. } = &input.action {
        if state.webhook_mgr.validate_url(webhook_url).await.is_err() {
            return StatusCode::BAD_REQUEST.into_response();
        }
    }
    if target != format!("/rooms/{room}/subscriptions/{subscription}/lifecycle") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    match state.store.mutate_seated_subscription(
        &room,
        &subscription,
        cert,
        method.as_str(),
        target,
        &body,
        &projection,
        &signature,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(result) => {
            state.webhook_mgr.wake();
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(crate::store::StoreError::MessageConflict | crate::store::StoreError::SeatedReplay) => {
            StatusCode::CONFLICT.into_response()
        }
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn history(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    // History signs an empty body. Reject unexpected bytes instead of verifying
    // a different request from the one the HTTP peer actually sent.
    if !body.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let decode_header = |name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| B64.decode(value).ok())
    };
    let (Some(cert), Some(projection), Some(signature)) = (
        headers
            .get("x-cowchat-certificate")
            .and_then(|value| value.to_str().ok()),
        decode_header("x-cowchat-request"),
        decode_header("x-cowchat-signature"),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if query.after < 0 || query.limit == 0 || query.limit > 100 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let target = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or(uri.path());
    match state.store.read_seated_history(
        &room,
        cert,
        method.as_str(),
        target,
        &projection,
        &signature,
        &query,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(crate::store::StoreError::SeatedReplay) => StatusCode::CONFLICT.into_response(),
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyEnvelope {
    pub publisher_cert: String,
    pub transport_generation: u64,
    pub scope: String,
    pub wrapped: String,
    pub signature: String,
}

pub(crate) async fn key_envelope(
    State(state): State<crate::web::AppState>,
    Path((room, recipient, generation)): Path<(String, String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let single = |name| {
        let mut values = headers.get_all(name).iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() {
            return None;
        }
        Some(value)
    };
    let (Some(cert), Some(projection), Some(signature)) = (
        single("x-cowchat-certificate"),
        single("x-cowchat-request").and_then(|v| B64.decode(v).ok()),
        single("x-cowchat-signature").and_then(|v| B64.decode(v).ok()),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    let result = state.store.seated_key_envelope(
        &room,
        &recipient,
        &generation,
        cert,
        method.as_str(),
        target,
        &body,
        &projection,
        &signature,
        chrono::Utc::now().timestamp_millis(),
    );
    let mut response = match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(crate::store::StoreError::SeatedReplay | crate::store::StoreError::MessageConflict) => {
            StatusCode::CONFLICT.into_response()
        }
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

pub(crate) async fn enroll_builder(
    State(state): State<crate::web::AppState>,
    Path(room): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let decode = |name| {
        let mut values = headers.get_all(name).iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() {
            return None;
        }
        B64.decode(value).ok()
    };
    let (Some(projection), Some(signature)) =
        (decode("x-cowchat-request"), decode("x-cowchat-signature"))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    match state.store.enroll_seated_builder(
        &room,
        target,
        &body,
        &projection,
        &signature,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok((seat, cert)) => (
            StatusCode::OK,
            Json(serde_json::json!({"room_id":room,"seat":seat,"cert":cert,"mode":"seated"})),
        )
            .into_response(),
        Err(crate::store::StoreError::MessageConflict | crate::store::StoreError::SeatedReplay) => {
            StatusCode::CONFLICT.into_response()
        }
        Err(crate::store::StoreError::SeatedAuthorization) => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
