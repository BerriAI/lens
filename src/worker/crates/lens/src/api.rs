use std::sync::Arc;

use axum::Router;
use lens_auth::{Authentication, Settings};
use lens_server::datasets::DatasetConfig;
use litellm_storage_clickhouse::{
    datasets::Datasets, evals::EvalStore, sessions::Sessions, state::ClickHouseState,
};
use litellm_traces_clickhouse::evals::EvalTraces;
use tokio::task::JoinHandle;

use crate::{Error, State, eval_judge::GatewayJudge, eval_runtime};

pub struct EvalConfig {
    pub public_url: String,
    pub judge: GatewayJudge,
}

pub async fn router(
    state: &State,
    settings: Settings,
    datasets: DatasetConfig,
    evals: EvalConfig,
) -> Result<(Router, JoinHandle<()>), Error> {
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
    let traces = EvalTraces::new(state.storage.client.clone(), connection.reader().clone());
    let task = eval_runtime::start(EvalStore::new(store.clone()), traces.clone(), evals.judge);
    let api = lens_server::sessions::router_with_auth(authentication.clone())
        .merge(lens_server::datasets::router(
            authentication.clone(),
            Datasets(store.clone()),
            state.storage.dataset_reader(store.clone()),
            datasets,
        ))
        .merge(lens_server::evals::router_with_traces(
            authentication,
            store,
            evals.public_url,
            traces,
        ));
    Ok((api, task))
}
