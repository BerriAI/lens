use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::{Connection, PgConnection, PgExecutor};

use crate::{Error, LegacySnapshot};

async fn rows<T: DeserializeOwned>(
    connection: impl PgExecutor<'_>,
    query: &'static str,
) -> Result<Vec<T>, Error> {
    sqlx::query_scalar::<_, Value>(query)
        .fetch_all(connection)
        .await?
        .into_iter()
        .map(|value| serde_json::from_value(value).map_err(Error::from))
        .collect()
}

pub async fn read_source(url: &str) -> Result<LegacySnapshot, Error> {
    let mut connection = PgConnection::connect(url).await?;
    let mut transaction = connection.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await?;
    let snapshot = LegacySnapshot {
        lenses: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_Lens\" AS row ORDER BY id").await?,
        runs: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensRun\" AS row ORDER BY lens_id, id").await?,
        reviews: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensReview\" AS row ORDER BY lens_id, criteria_key, execution_id").await?,
        workers: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensWorker\" AS row ORDER BY id").await?,
        ingestion_keys: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensIngestionKey\" AS row ORDER BY id").await?,
        datasets: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensDataset\" AS row ORDER BY id, revision").await?,
        signal_configs: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensSignalConfig\" AS row ORDER BY id").await?,
        trace_signals: rows(&mut *transaction, "SELECT to_jsonb(row) FROM \"LiteLLM_LensTraceSignal\" AS row ORDER BY trace_id, trace_ref").await?,
    };
    transaction.commit().await?;
    Ok(snapshot)
}
