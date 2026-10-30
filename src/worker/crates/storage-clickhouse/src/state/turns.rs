use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError, Weak},
};

use tokio::sync::{Mutex as Queue, OwnedMutexGuard};

#[derive(Clone, Default)]
pub(super) struct Turns(Arc<Mutex<HashMap<String, Weak<Queue<()>>>>>);

impl Turns {
    pub(super) async fn take(&self, key: &str) -> OwnedMutexGuard<()> {
        self.queue(key).lock_owned().await
    }

    fn queue(&self, key: &str) -> Arc<Queue<()>> {
        let mut queues = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        queues.retain(|_, queue| queue.strong_count() > 0);
        if let Some(queue) = queues.get(key).and_then(Weak::upgrade) {
            return queue;
        }
        let queue = Arc::new(Queue::new(()));
        queues.insert(key.to_owned(), Arc::downgrade(&queue));
        queue
    }
}
