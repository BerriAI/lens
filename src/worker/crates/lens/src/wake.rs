use std::{sync::Arc, time::Duration};

use tokio::sync::watch;

#[derive(Clone)]
pub struct Wake(Arc<watch::Sender<u64>>);

impl Default for Wake {
    fn default() -> Self {
        Self(Arc::new(watch::channel(0).0))
    }
}

impl Wake {
    pub fn notify(&self) {
        self.0.send_modify(|count| *count = count.wrapping_add(1));
    }

    pub fn listen(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

pub async fn arrival(
    wake: &mut watch::Receiver<u64>,
    settle: Duration,
    fallback: Duration,
) -> bool {
    let woken = tokio::select! {
        changed = wake.changed() => changed.is_ok(),
        () = tokio::time::sleep(fallback) => false,
    };
    if woken {
        tokio::time::sleep(settle).await;
    }
    wake.borrow_and_update();
    woken
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tokio::time::Instant;

    const SETTLE: Duration = Duration::from_secs(5);
    const FALLBACK: Duration = Duration::from_secs(60);

    #[rstest]
    #[case::trace_arrives(Some(Duration::from_secs(7)), true, Duration::from_secs(12))]
    #[case::nothing_arrives(None, false, FALLBACK)]
    #[tokio::test(start_paused = true)]
    async fn work_starts_after_the_quiet_period_or_the_fallback_sweep(
        #[case] arrives_after: Option<Duration>,
        #[case] woken: bool,
        #[case] waited: Duration,
    ) {
        let wake = Wake::default();
        let mut listener = wake.listen();
        let started = Instant::now();
        if let Some(delay) = arrives_after {
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                wake.notify();
            });
        }

        assert_eq!(arrival(&mut listener, SETTLE, FALLBACK).await, woken);
        assert_eq!(started.elapsed(), waited);
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn batches_arriving_during_the_quiet_period_share_one_run() {
        let wake = Wake::default();
        let mut listener = wake.listen();
        wake.notify();
        let later = wake.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            later.notify();
        });
        arrival(&mut listener, SETTLE, FALLBACK).await;
        let started = Instant::now();

        assert!(!arrival(&mut listener, SETTLE, FALLBACK).await);
        assert_eq!(started.elapsed(), FALLBACK);
    }
}
