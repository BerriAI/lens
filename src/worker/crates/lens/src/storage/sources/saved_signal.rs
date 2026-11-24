use lens_signals::{SignalRepository, config_key};
use litellm_storage_clickhouse::{Query, signals::Signals, state::ClickHouseState};
use litellm_traces_clickhouse::query::lens::{LensSample, LensSampleParams, LensSampleRow};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use super::SourceReader;
use crate::Error;

#[derive(Serialize)]
struct Parameters {
    #[serde(flatten)]
    scope: LensSampleParams,
    signal_id: String,
    signal_config: String,
    signal_threshold: f64,
}

struct SavedSignalSample;

#[derive(Deserialize)]
struct SavedSignalRow {
    #[serde(flatten)]
    sample: LensSampleRow,
    signal_snapshot: String,
}

impl Query for SavedSignalSample {
    type Params = Parameters;
    type Row = SavedSignalRow;

    const SQL: &'static str = include_str!("saved_signal.sql");

    fn sql() -> Cow<'static, str> {
        Cow::Owned(LensSample::filtered_sql(Self::SQL))
    }
}

impl SourceReader {
    pub(super) async fn saved_signal_sample(
        &self,
        scope: LensSampleParams,
        signal_id: &str,
    ) -> Result<Vec<LensSampleRow>, Error> {
        self.0.require_storage()?;
        let (snapshot, after) = match scope.after.split_once(':') {
            Some((snapshot, after)) => (Some(snapshot.to_owned()), after.to_owned()),
            None if scope.after.is_empty() => (None, String::new()),
            None => return Err(Error::InvalidRequest),
        };
        let connection = self.0.storage.config.storage().reader();
        let repository = Signals(ClickHouseState::new(
            self.0.storage.client.clone(),
            connection.clone(),
        ));
        let config = repository
            .get_config()
            .await
            .map_err(|_| Error::Unavailable)?;
        if !config.signals.iter().any(|signal| signal.id == signal_id) {
            return if snapshot.is_some() {
                Err(Error::Unavailable)
            } else {
                Ok(Vec::new())
            };
        }
        let parameters = Parameters {
            signal_id: signal_id.to_owned(),
            signal_config: config_key(&config),
            signal_threshold: config.threshold,
            scope: LensSampleParams { after, ..scope },
        };
        let rows = self.read::<SavedSignalSample>(&parameters).await?;
        if snapshot.is_some_and(|snapshot| {
            rows.first()
                .is_none_or(|row| row.signal_snapshot != snapshot)
        }) {
            return Err(Error::Unavailable);
        }
        Ok(rows
            .into_iter()
            .map(|row| LensSampleRow {
                selection_key: format!("{}:{}", row.signal_snapshot, row.sample.selection_key),
                ..row.sample
            })
            .collect())
    }
}
