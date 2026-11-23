use lens_signals::{SignalRepository, config_key};
use litellm_storage_clickhouse::{Query, execute_read, signals::Signals, state::ClickHouseState};
use litellm_traces_clickhouse::query::lens::{LensSample, LensSampleParams, LensSampleRow};
use serde::{Deserialize, Serialize};

use super::SourceReader;
use crate::{Error, wait_for_read_slot};

#[derive(Serialize)]
struct Parameters<'a> {
    #[serde(flatten)]
    scope: LensSampleParams,
    signal_id: &'a str,
    signal_config: String,
    signal_threshold: f64,
    signal_sample_cap: u64,
    signal_sample_percent: f64,
    signal_preview: u8,
    signal_after: String,
    signal_limit: u32,
    signal_offset: u64,
}

#[derive(Deserialize)]
struct Rows {
    data: Vec<LensSampleRow>,
}

impl SourceReader {
    pub(super) async fn saved_signal_sample(
        &self,
        scope: LensSampleParams,
        signal_id: &str,
    ) -> Result<Vec<LensSampleRow>, Error> {
        self.0.require_storage()?;
        let _permit = wait_for_read_slot(self.0.read_slots.clone().acquire_owned()).await?;
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
            return Ok(Vec::new());
        }
        let parameters = Parameters {
            signal_id,
            signal_config: config_key(&config),
            signal_threshold: config.threshold,
            signal_sample_cap: scope.sample_cap,
            signal_sample_percent: scope.sample_percent,
            signal_preview: scope.preview,
            signal_after: scope.after.clone(),
            signal_limit: scope.limit,
            signal_offset: scope.offset,
            scope: LensSampleParams {
                sample_cap: 0,
                sample_percent: 100.0,
                preview: 1,
                after: String::new(),
                limit: u32::MAX,
                offset: 0,
                ..scope
            },
        };
        let sql = format!(
            "WITH scoped AS ({}) {}",
            LensSample::SQL,
            include_str!("saved_signal.sql")
        );
        let bound = serde_json::from_value(serde_json::to_value(parameters)?)?;
        let body = execute_read(&self.0.storage.client, connection, &sql, &bound).await?;
        Ok(serde_json::from_str::<Rows>(&body)?.data)
    }
}
