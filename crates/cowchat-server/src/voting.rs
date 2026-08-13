use cowchat_core::*;
use dashmap::DashMap;
use rand::seq::SliceRandom;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::broker::Broker;
use crate::store::Store;

/// Tracks active votes (open votes awaiting completion).
#[derive(Debug, Clone)]
pub struct ActiveVote {
    pub vote_id: String,
    pub room_id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub created_by: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub closes_at: Option<chrono::DateTime<chrono::Utc>>,
    pub ballots: Vec<(String, String, usize)>, // (agent_id, agent_name, option_index)
    pub eligible_voters: usize,
    pub eligible_agents: HashSet<String>,
}

/// Tracks an active election (in the nomination window).
#[derive(Debug, Clone)]
pub struct ActiveElection {
    pub room_id: String,
    pub candidates: Vec<String>, // agent_ids
    pub declined: HashSet<String>,
    pub started_by: String,
}

/// In-memory state for voting and elections.
pub struct VoteManager {
    /// Active (open) votes: vote_id -> ActiveVote
    pub active_votes: Arc<DashMap<String, ActiveVote>>,
    /// Active elections (during the opt-out window): room_id -> ActiveElection
    pub active_elections: Arc<DashMap<String, ActiveElection>>,
    /// Current room leaders: room_id -> agent_id
    pub room_leaders: Arc<DashMap<String, String>>,
}

impl VoteManager {
    pub fn new(store: Arc<Store>, broker: Arc<Broker>) -> Self {
        let manager = Self {
            active_votes: Arc::new(DashMap::new()),
            active_elections: Arc::new(DashMap::new()),
            room_leaders: Arc::new(DashMap::new()),
        };
        if let Ok(votes) = store.list_open_votes() {
            for meta in votes {
                let ballots = store.get_vote_ballots(&meta.vote_id).unwrap_or_default();
                let eligible_agents = store
                    .get_vote_eligible_agents(&meta.vote_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect::<HashSet<_>>();
                let vote = ActiveVote {
                    vote_id: meta.vote_id.clone(),
                    room_id: meta.room_id,
                    title: meta.title,
                    description: meta.description,
                    options: meta.options,
                    created_by: meta.created_by,
                    created_at: meta.created_at,
                    closes_at: meta.closes_at,
                    ballots,
                    eligible_voters: meta.eligible_voters,
                    eligible_agents,
                };
                manager.active_votes.insert(meta.vote_id.clone(), vote);
                if let Some(closes_at) = meta.closes_at {
                    let delay = (closes_at - chrono::Utc::now())
                        .to_std()
                        .unwrap_or(Duration::ZERO);
                    manager.schedule_deadline(meta.vote_id, delay, broker.clone(), store.clone());
                }
            }
        }
        manager
    }

    fn schedule_deadline(
        &self,
        vote_id: String,
        delay: Duration,
        broker: Arc<Broker>,
        store: Arc<Store>,
    ) {
        let active_votes = self.active_votes.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some((_, vote)) = active_votes.remove(&vote_id) {
                close_and_broadcast_vote(vote, &broker, &store).await;
            }
        });
    }

    /// Create a vote and optionally spawn a deadline timer.
    pub fn create_vote(
        &self,
        vote_id: String,
        room_id: String,
        title: String,
        description: Option<String>,
        options: Vec<String>,
        created_by: String,
        duration_secs: Option<u64>,
        eligible_agents: Vec<String>,
        broker: Arc<Broker>,
        store: Arc<Store>,
    ) -> ActiveVote {
        let now = chrono::Utc::now();
        let closes_at = duration_secs.map(|s| now + chrono::Duration::seconds(s as i64));

        let vote = ActiveVote {
            vote_id: vote_id.clone(),
            room_id: room_id.clone(),
            title,
            description,
            options,
            created_by,
            created_at: now,
            closes_at,
            ballots: Vec::new(),
            eligible_voters: eligible_agents.len(),
            eligible_agents: eligible_agents.into_iter().collect(),
        };

        self.active_votes.insert(vote_id.clone(), vote.clone());

        // Spawn deadline timer if duration is set
        if let Some(secs) = duration_secs {
            self.schedule_deadline(vote_id.clone(), Duration::from_secs(secs), broker, store);
        }

        vote
    }

    /// Record a ballot. Returns (votes_cast, eligible). Triggers close if all voted.
    pub async fn cast_vote(
        &self,
        vote_id: &str,
        agent_id: &str,
        agent_name: &str,
        option_index: usize,
        broker: &Arc<Broker>,
        store: &Arc<Store>,
    ) -> Result<(usize, usize), ErrorCode> {
        let mut should_close = false;
        let votes_cast;
        let eligible;

        {
            let mut vote = self
                .active_votes
                .get_mut(vote_id)
                .ok_or(ErrorCode::VoteNotFound)?;

            if option_index >= vote.options.len() {
                return Err(ErrorCode::InvalidOption);
            }

            if !vote.eligible_agents.contains(agent_id) {
                return Err(ErrorCode::AccessDenied);
            }

            // Check not already voted
            if vote.ballots.iter().any(|(id, _, _)| id == agent_id) {
                return Err(ErrorCode::AlreadyVoted);
            }

            vote.ballots
                .push((agent_id.to_string(), agent_name.to_string(), option_index));

            votes_cast = vote.ballots.len();
            eligible = vote.eligible_voters;

            if votes_cast >= eligible {
                should_close = true;
            }
        }

        if should_close {
            if let Some((_, vote)) = self.active_votes.remove(vote_id) {
                close_and_broadcast_vote(vote, broker, store).await;
            }
        }

        Ok((votes_cast, eligible))
    }

    /// Start a leader election in a room.
    pub fn start_election(
        &self,
        room_id: &str,
        candidates: Vec<String>,
        started_by: &str,
        broker: Arc<Broker>,
    ) -> Result<(), ErrorCode> {
        if self.active_elections.contains_key(room_id) {
            return Err(ErrorCode::ElectionInProgress);
        }

        let election = ActiveElection {
            room_id: room_id.to_string(),
            candidates: candidates.clone(),
            declined: HashSet::new(),
            started_by: started_by.to_string(),
        };

        self.active_elections.insert(room_id.to_string(), election);

        // Spawn timer: after 2 seconds, pick the leader
        let active_elections = self.active_elections.clone();
        let room_leaders = self.room_leaders.clone();
        let room_id = room_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if broker.is_room_destroyed(&room_id) {
                active_elections.remove(&room_id);
                room_leaders.remove(&room_id);
                return;
            }
            if let Some((_, election)) = active_elections.remove(&room_id) {
                let remaining: Vec<&String> = election
                    .candidates
                    .iter()
                    .filter(|c| !election.declined.contains(*c))
                    .collect();

                if remaining.is_empty() {
                    // Everyone declined -- no leader
                    let event = Frame::event(
                        FrameType::LeaderCleared,
                        serde_json::json!({"room_id": room_id, "reason": "all candidates declined"}),
                    );
                    broker.broadcast_to_room_all(&room_id, &event);
                    return;
                }

                let mut rng = rand::thread_rng();
                let winner = remaining.choose(&mut rng).unwrap();
                let winner_id = (*winner).clone();

                // Get winner name
                let winner_name = broker
                    .agents
                    .get(&winner_id)
                    .map(|a| a.info.name.clone())
                    .unwrap_or_else(|| winner_id.clone());

                if broker.is_room_destroyed(&room_id) {
                    room_leaders.remove(&room_id);
                    return;
                }
                room_leaders.insert(room_id.clone(), winner_id.clone());

                let event = Frame::event(
                    FrameType::LeaderElected,
                    serde_json::json!({
                        "room_id": room_id,
                        "leader_id": winner_id,
                        "leader_name": winner_name,
                    }),
                );
                broker.broadcast_to_room_all(&room_id, &event);
                log::info!(
                    "Leader elected in {}: {} ({})",
                    room_id,
                    winner_name,
                    winner_id
                );
            }
        });

        Ok(())
    }

    /// Agent opts out of an active election.
    pub fn decline_election(&self, room_id: &str, agent_id: &str) -> Result<(), ErrorCode> {
        let mut election = self
            .active_elections
            .get_mut(room_id)
            .ok_or(ErrorCode::NoElectionActive)?;
        election.declined.insert(agent_id.to_string());
        Ok(())
    }

    /// Clear leadership for a room (called when leader disconnects/leaves).
    pub fn clear_leader(&self, room_id: &str, broker: &Broker) {
        if let Some((_, _)) = self.room_leaders.remove(room_id) {
            let event = Frame::event(
                FrameType::LeaderCleared,
                serde_json::json!({"room_id": room_id, "reason": "leader left"}),
            );
            broker.broadcast_to_room_all(room_id, &event);
        }
    }

    /// Check if an agent is the leader of a room.
    pub fn is_leader(&self, room_id: &str, agent_id: &str) -> bool {
        self.room_leaders
            .get(room_id)
            .map(|leader| leader.value() == agent_id)
            .unwrap_or(false)
    }

    /// Get the current leader of a room.
    pub fn get_leader(&self, room_id: &str) -> Option<String> {
        self.room_leaders.get(room_id).map(|v| v.value().clone())
    }

    /// Clear leadership if a specific agent was the leader (on disconnect).
    pub fn clear_leader_if_agent(&self, agent_id: &str, broker: &Broker) {
        let rooms_to_clear: Vec<String> = self
            .room_leaders
            .iter()
            .filter(|entry| entry.value() == agent_id)
            .map(|entry| entry.key().clone())
            .collect();

        for room_id in rooms_to_clear {
            self.clear_leader(&room_id, broker);
        }
    }

    /// Remove every active coordination object belonging to a destroyed room.
    /// Deadline/election tasks share these maps, so once entries are removed
    /// their delayed callbacks become no-ops and cannot publish late events.
    pub fn forget_room(&self, room_id: &str) {
        self.active_votes
            .retain(|_, vote| vote.room_id.as_str() != room_id);
        self.active_elections.remove(room_id);
        self.room_leaders.remove(room_id);
    }
}

/// Close a vote and broadcast the results to the room.
async fn close_and_broadcast_vote(vote: ActiveVote, broker: &Arc<Broker>, store: &Arc<Store>) {
    // Build tally
    let mut tally_counts = vec![0usize; vote.options.len()];
    let mut ballot_entries = Vec::new();

    for (agent_id, agent_name, option_index) in &vote.ballots {
        if *option_index < tally_counts.len() {
            tally_counts[*option_index] += 1;
        }
        ballot_entries.push(BallotEntry {
            agent_id: agent_id.clone(),
            agent_name: agent_name.clone(),
            option_index: *option_index,
        });
    }

    let tally: Vec<VoteTally> = vote
        .options
        .iter()
        .enumerate()
        .map(|(i, text)| VoteTally {
            option_index: i,
            option_text: text.clone(),
            count: tally_counts[i],
        })
        .collect();

    let result = VoteResultPayload {
        vote_id: vote.vote_id.clone(),
        room_id: vote.room_id.clone(),
        title: vote.title.clone(),
        options: vote.options.clone(),
        tally,
        ballots: ballot_entries,
        total_votes: vote.ballots.len(),
        eligible_voters: vote.eligible_voters,
    };

    let _ = store.close_vote(&vote.vote_id);

    let event = Frame::event(
        FrameType::VoteResult,
        serde_json::to_value(&result).unwrap(),
    );
    broker.broadcast_to_room_all(&vote.room_id, &event);

    log::info!(
        "Vote '{}' closed in room {}: {} votes cast",
        vote.title,
        vote.room_id,
        vote.ballots.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn destroyed_room_coordination_state_is_removed() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(Broker::new(
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        ));
        let manager = VoteManager::new(store.clone(), broker.clone());
        manager.create_vote(
            "vote".into(),
            "doomed".into(),
            "Vote".into(),
            None,
            vec!["yes".into(), "no".into()],
            "creator".into(),
            None,
            vec!["creator".into()],
            broker,
            store,
        );
        manager.active_elections.insert(
            "doomed".into(),
            ActiveElection {
                room_id: "doomed".into(),
                candidates: vec!["creator".into()],
                declined: HashSet::new(),
                started_by: "creator".into(),
            },
        );
        manager
            .room_leaders
            .insert("doomed".into(), "creator".into());

        manager.forget_room("doomed");
        assert!(manager.active_votes.is_empty());
        assert!(!manager.active_elections.contains_key("doomed"));
        assert!(!manager.room_leaders.contains_key("doomed"));
    }
}
