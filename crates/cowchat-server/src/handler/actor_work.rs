use super::*;

pub(super) async fn handle(
    frame: &Frame,
    agent_id: &str,
    agent_api_key: &str,
    no_auth: bool,
    store: &Arc<Store>,
    broker: &Arc<Broker>,
    webhook_mgr: &Arc<crate::webhooks::WebhookManager>,
) -> Frame {
    let req_id = frame.id.as_deref();
    let invalid = |message: String| {
        Frame::error(
            req_id,
            ErrorPayload::new(ErrorCode::InvalidPayload, message),
        )
    };
    let owner = if no_auth { "no-auth" } else { agent_api_key };
    if frame.frame_type == FrameType::SubscribeActor {
        let p: SubscribeActorPayload = match serde_json::from_value(frame.payload.clone()) {
            Ok(p) => p,
            Err(e) => return invalid(e.to_string()),
        };
        if let Err(e) = authorize_room(req_id, &p.room_id, agent_api_key, no_auth, store) {
            return e;
        }
        if p.secret.is_empty() {
            return invalid("secret is required".into());
        }
        if let Err(e) = webhook_mgr.validate_url(&p.webhook_url).await {
            return invalid(e);
        }
        let id = match store.create_actor_subscription(
            &p.room_id,
            owner,
            agent_id,
            &p.webhook_url,
            &p.secret,
            p.mode,
        ) {
            Ok(id) => id,
            Err(e) => return invalid(e.to_string()),
        };
        if broker.is_room_destroyed(&p.room_id) {
            let _ = store.delete_subscription(&id, owner);
            return invalid("room was destroyed".into());
        }
        return Frame::ok(req_id, serde_json::json!({"subscription_id": id}));
    }
    let p: ClaimActorWorkPayload = match serde_json::from_value(frame.payload.clone()) {
        Ok(p) => p,
        Err(e) => return invalid(e.to_string()),
    };
    let sub = match store.get_subscription(&p.subscription_id) {
        Ok(Some((sub, key, _))) if key == owner => sub,
        _ => return invalid("subscription not found".into()),
    };
    if let Err(e) = authorize_room(req_id, &sub.room_id, agent_api_key, no_auth, store) {
        return e;
    }
    if frame.frame_type == FrameType::ClaimActorWork {
        match store.claim_actor_work(&p.subscription_id, agent_id, chrono::Utc::now().timestamp()) {
            Ok(work) => Frame::ok(req_id, serde_json::json!({"work": work})),
            Err(e) => invalid(e.to_string()),
        }
    } else {
        let p: CompleteActorWorkPayload = match serde_json::from_value(frame.payload.clone()) {
            Ok(p) => p,
            Err(e) => return invalid(e.to_string()),
        };
        match store.complete_actor_work(&p.subscription_id, &p.work_id, agent_id, p.outcome) {
            Ok(()) => {
                webhook_mgr.wake();
                Frame::ok(req_id, serde_json::json!({"completed": true}))
            }
            Err(e) => invalid(e.to_string()),
        }
    }
}
