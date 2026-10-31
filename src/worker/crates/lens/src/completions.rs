use futures_util::{
    Stream, StreamExt,
    future::{self, Either},
};
use std::{collections::VecDeque, future::Future};

pub(crate) struct Completions<S: Stream> {
    calls: S,
    ready: VecDeque<S::Item>,
}

impl<S: Stream + Unpin> Completions<S> {
    pub(crate) fn new(calls: S) -> Self {
        Self {
            calls,
            ready: VecDeque::new(),
        }
    }

    pub(crate) async fn next(&mut self) -> Option<S::Item> {
        match self.ready.pop_front() {
            Some(item) => Some(item),
            None => self.calls.next().await,
        }
    }

    pub(crate) async fn during<T>(&mut self, work: impl Future<Output = T>) -> T {
        let mut work = std::pin::pin!(work);
        loop {
            match future::select(work.as_mut(), self.calls.next()).await {
                Either::Left((output, _)) => return output,
                Either::Right((Some(item), _)) => self.ready.push_back(item),
                Either::Right((None, _)) => return work.await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::{pin::Pin, time::Duration};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn calls_in_flight_keep_running_while_the_caller_waits_on_them() {
        let finished = Notify::new();
        let calls: Vec<Pin<Box<dyn Future<Output = &str> + Send + '_>>> = vec![
            Box::pin(async { "first" }),
            Box::pin(async {
                tokio::task::yield_now().await;
                finished.notify_one();
                "second"
            }),
        ];
        let mut completions = Completions::new(stream::iter(calls).buffer_unordered(2));
        let first = completions.next().await;
        tokio::time::timeout(
            Duration::from_secs(5),
            completions.during(finished.notified()),
        )
        .await
        .unwrap();
        assert_eq!(
            (first, completions.next().await, completions.next().await),
            (Some("first"), Some("second"), None)
        );
    }
}
