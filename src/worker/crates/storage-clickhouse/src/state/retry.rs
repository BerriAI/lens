use std::time::Duration;

use rand::Rng;

fn ceiling(attempt: u64) -> u64 {
    20 * (attempt + 1).min(8)
}

pub(crate) async fn backoff(attempt: u64) {
    let delay = rand::thread_rng().gen_range(0..=ceiling(attempt));
    tokio::time::sleep(Duration::from_millis(delay)).await;
}

#[cfg(test)]
mod tests {
    use super::ceiling;
    use rstest::rstest;

    #[rstest]
    #[case::first_conflict(0, 20)]
    #[case::second_conflict(1, 40)]
    #[case::last_uncapped_conflict(6, 140)]
    #[case::cap(7, 160)]
    #[case::past_cap(39, 160)]
    fn conflict_retry_delay_is_bounded(#[case] attempt: u64, #[case] expected: u64) {
        assert_eq!(ceiling(attempt), expected);
    }
}
