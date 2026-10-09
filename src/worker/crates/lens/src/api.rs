use std::sync::Arc;

use axum::Router;
use lens_auth::{Authentication, Settings};
use lens_server::datasets::DatasetConfig;
use litellm_storage_clickhouse::{datasets::Datasets, sessions::Sessions, state::ClickHouseState};

use crate::{Error, State};

pub async fn router(
    state: &State,
    settings: Settings,
    datasets: DatasetConfig,
) -> Result<Router, Error> {
    state.storage.ensure_schema().await?;
    let connection = state.storage.config.storage();
    let store = ClickHouseState::new(state.storage.client.clone(), connection.reader().clone());
    store
        .initialize(&format!("/lens/{}", connection.database()))
        .await?;
    let authentication = Arc::new(Authentication {
        settings,
        sessions: Sessions(store.clone()),
    });
    Ok(
        lens_server::sessions::router_with_auth(authentication.clone()).merge(
            lens_server::datasets::router(
                authentication,
                Datasets(store.clone()),
                state.storage.dataset_reader(store),
                datasets,
            ),
        ),
    )
}
