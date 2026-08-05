use chrono::Utc;
use cowchat_core::TaskInfo;
use dashmap::DashMap;
use std::sync::Arc;

use crate::store::Store;
use crate::store::StoreError;

/// In-memory task tracking for rooms. Tasks are lightweight work items that
/// agents can assign, update, and query — filling the gap between unstructured
/// chat messages and formal votes.
pub struct TaskManager {
    /// task_id -> TaskInfo
    pub tasks: Arc<DashMap<String, TaskInfo>>,
    store: Arc<Store>,
}

impl TaskManager {
    pub fn new(store: Arc<Store>) -> Self {
        let tasks = Arc::new(DashMap::new());
        if let Ok(persisted) = store.load_tasks() {
            for task in persisted {
                tasks.insert(task.task_id.clone(), task);
            }
        }
        Self { tasks, store }
    }

    /// Create a new task in a room.
    pub fn create_task(
        &self,
        task_id: String,
        room_id: String,
        title: String,
        description: Option<String>,
        assignee: Option<String>,
        created_by: String,
    ) -> Result<TaskInfo, StoreError> {
        let now = Utc::now();
        let task = TaskInfo {
            task_id: task_id.clone(),
            room_id,
            title,
            description,
            status: "pending".to_string(),
            assignee,
            created_by,
            created_at: now,
            updated_at: None,
            note: None,
        };
        self.store.save_task(&task)?;
        self.tasks.insert(task_id, task.clone());
        Ok(task)
    }

    /// Update a task's status, assignee, and/or note. Returns the updated task.
    pub fn update_task(
        &self,
        task_id: &str,
        status: Option<String>,
        assignee: Option<String>,
        note: Option<String>,
    ) -> Result<Option<TaskInfo>, StoreError> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Ok(None);
        };
        let mut task = entry.value().clone();

        if let Some(s) = status {
            task.status = s;
        }
        if let Some(a) = assignee {
            task.assignee = Some(a);
        }
        if let Some(n) = note {
            task.note = Some(n);
        }
        task.updated_at = Some(Utc::now());

        self.store.save_task(&task)?;
        *entry.value_mut() = task.clone();
        Ok(Some(task))
    }

    /// List tasks in a room, optionally filtered by status.
    pub fn list_tasks(&self, room_id: &str, status_filter: Option<&str>) -> Vec<TaskInfo> {
        self.tasks
            .iter()
            .filter(|entry| {
                let t = entry.value();
                t.room_id == room_id && status_filter.map(|s| t.status == s).unwrap_or(true)
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get a single task by ID.
    pub fn get_task(&self, task_id: &str) -> Option<TaskInfo> {
        self.tasks.get(task_id).map(|t| t.value().clone())
    }

    /// Drop all in-memory tasks for a room after its durable rows have been
    /// deleted transactionally by the store.
    pub fn forget_room(&self, room_id: &str) {
        self.tasks.retain(|_, task| task.room_id != room_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_survive_manager_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
        let manager = TaskManager::new(store.clone());
        manager
            .create_task(
                "task-1".into(),
                "lobby".into(),
                "Persist me".into(),
                None,
                Some("agent".into()),
                "creator".into(),
            )
            .unwrap();
        manager
            .update_task("task-1", Some("in_progress".into()), None, None)
            .unwrap();
        drop(manager);

        let restored = TaskManager::new(store);
        let task = restored.get_task("task-1").expect("task must reload");
        assert_eq!(task.status, "in_progress");
        assert_eq!(task.assignee.as_deref(), Some("agent"));
    }

    #[test]
    fn persistence_failure_does_not_acknowledge_or_mutate_task_state() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = TaskManager::new(store);
        let create = manager.create_task(
            "bad-create".into(),
            "missing-room".into(),
            "Must fail".into(),
            None,
            None,
            "creator".into(),
        );
        assert!(create.is_err());
        assert!(manager.get_task("bad-create").is_none());

        let now = Utc::now();
        manager.tasks.insert(
            "bad-update".into(),
            TaskInfo {
                task_id: "bad-update".into(),
                room_id: "missing-room".into(),
                title: "Must stay pending".into(),
                description: None,
                status: "pending".into(),
                assignee: None,
                created_by: "creator".into(),
                created_at: now,
                updated_at: None,
                note: None,
            },
        );
        let update = manager.update_task(
            "bad-update",
            Some("completed".into()),
            None,
            Some("not persisted".into()),
        );
        assert!(update.is_err());
        assert_eq!(manager.get_task("bad-update").unwrap().status, "pending");
    }

    #[test]
    fn concurrent_disjoint_updates_are_serialized_and_merged() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = Arc::new(TaskManager::new(store));
        manager
            .create_task(
                "concurrent".into(),
                "lobby".into(),
                "Merge updates".into(),
                None,
                None,
                "creator".into(),
            )
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let status_worker = {
            let manager = manager.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                manager
                    .update_task("concurrent", Some("completed".into()), None, None)
                    .unwrap();
            })
        };
        let note_worker = {
            let manager = manager.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                manager
                    .update_task(
                        "concurrent",
                        None,
                        Some("agent".into()),
                        Some("done".into()),
                    )
                    .unwrap();
            })
        };
        barrier.wait();
        status_worker.join().unwrap();
        note_worker.join().unwrap();
        let task = manager.get_task("concurrent").unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.assignee.as_deref(), Some("agent"));
        assert_eq!(task.note.as_deref(), Some("done"));
    }

    #[test]
    fn destroyed_room_tasks_are_removed_from_memory() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .create_room_with_visibility(
                "doomed",
                "doomed",
                None,
                None,
                Some("creator"),
                "private",
                Some("key"),
                false,
            )
            .unwrap();
        let manager = TaskManager::new(store.clone());
        manager
            .create_task(
                "doomed-task".into(),
                "doomed".into(),
                "Remove me".into(),
                None,
                None,
                "creator".into(),
            )
            .unwrap();
        store
            .destroy_room_authorized("doomed", "creator", "key", false)
            .unwrap();
        manager.forget_room("doomed");
        assert!(manager.get_task("doomed-task").is_none());
    }
}
