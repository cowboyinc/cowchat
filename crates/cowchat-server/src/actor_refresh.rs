//! Background finalized-proof refresh. No message request waits on this worker.
use crate::{
    actor_proof::{ActorProofAuthority, VerifiedActorState},
    store::Store,
};
use std::{sync::Arc, time::Duration};

pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const PAGE_SIZE: i64 = 32;
const PARALLEL_FETCHES: usize = 4;

pub(crate) struct ActorRefreshTask(tokio::task::JoinHandle<()>);
impl Drop for ActorRefreshTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn start(store: Arc<Store>, authority: Arc<ActorProofAuthority>) -> ActorRefreshTask {
    ActorRefreshTask(tokio::spawn(async move {
        loop {
            if let Err(error) = refresh_once(store.clone(), authority.clone()).await {
                log::warn!("actor control refresh failed: {error}");
            }
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    }))
}

/// One bounded-memory sweep, four in-flight proof reads, no retry loop inside
/// an actor fetch. Failure leaves previously verified state unchanged.
pub(crate) async fn refresh_once(
    store: Arc<Store>,
    authority: Arc<ActorProofAuthority>,
) -> Result<(), crate::store::StoreError> {
    let mut after = None;
    loop {
        let actors = store.actors_for_refresh(after, PAGE_SIZE)?;
        if actors.is_empty() {
            break;
        }
        after = actors.last().copied();
        let mut tasks = tokio::task::JoinSet::new();
        for (chain, actor) in actors {
            let (store, authority) = (store.clone(), authority.clone());
            tasks.spawn(async move {
                // An untrusted courier cannot redirect this actor/chain task.
                let result = authority.fetch_state(actor).await;
                let now = chrono::Utc::now().timestamp_millis();
                let ingested = match result {
                    Ok(VerifiedActorState::Present(proof))
                        if i64::try_from(proof.chain_id()).ok() == Some(chain) =>
                    {
                        store.ingest_actor_control(&proof, now)
                    }
                    Ok(VerifiedActorState::Absent(proof))
                        if i64::try_from(proof.chain_id()).ok() == Some(chain) =>
                    {
                        store.ingest_actor_absence(&proof, now)
                    }
                    _ => {
                        log::warn!("actor control refresh: proof unavailable or invalid");
                        return;
                    }
                };
                if let Err(error) = ingested {
                    log::warn!("actor control refresh rejected: {error}");
                }
            });
            if tasks.len() >= PARALLEL_FETCHES {
                let _ = tasks.join_next().await;
            }
        }
        while tasks.join_next().await.is_some() {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actor_refresh_task_cancels_when_server_guard_drops() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let authority = Arc::new(crate::actor_proof::tests::authority(
            vec![],
            "http://127.0.0.1:1/proof/finalized-state".into(),
        ));
        let task = start(store, authority);
        let abort = task.0.abort_handle();
        tokio::task::yield_now().await;
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !abort.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
