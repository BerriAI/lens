use std::sync::Arc;

use axum::Router;
use lens_auth::{Authentication, Settings};
use lens_server::datasets::DatasetConfig;
use litellm_storage_clickhouse::{datasets::Datasets, sessions::Sessions, state::ClickHouseState};
use litellm_traces_clickhouse::evals::EvalTraces;

use crate::{Error, State};

pub async fn authentication(
    state: &State,
    settings: Settings,
) -> Result<Arc<Authentication<Sessions>>, Error> {
    state.storage.ensure_schema().await?;
    let connection = state.storage.config.storage();
    let store = ClickHouseState::new(state.storage.client.clone(), connection.reader().clone());
    store
        .initialize(&format!("/lens/{}", connection.database()))
        .await?;
    Ok(Arc::new(Authentication {
        settings,
        sessions: Sessions(store),
    }))
}

pub async fn router(
    state: &State,
    settings: Settings,
    datasets: DatasetConfig,
) -> Result<Router, Error> {
    let authentication = authentication(state, settings).await?;
    let store = authentication.sessions.0.clone();
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

pub fn router_with_evals(
    state: &State,
    authentication: Arc<Authentication<Sessions>>,
    datasets: DatasetConfig,
    public_url: String,
    traces: EvalTraces,
) -> Router {
    let store = authentication.sessions.0.clone();
    lens_server::sessions::router_with_auth(authentication.clone())
        .merge(lens_server::datasets::router_with_evals(
            authentication.clone(),
            Datasets(store.clone()),
            state.storage.dataset_reader(store.clone()),
            datasets,
            store.clone(),
            Some(traces.clone()),
        ))
        .merge(lens_server::evals::router_without_cases(
            authentication,
            store,
            public_url,
            Some(traces),
        ))
}
