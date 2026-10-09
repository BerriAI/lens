use litellm_lens::{
    State, Storage, api, auth,
    config::{Config, http_client},
    control::Control,
    provision, router,
    worker::Worker,
};
use std::{io::Write, sync::Arc, time::Duration};

struct Diagnostics;

impl litellm_tracing::Sink for Diagnostics {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target().starts_with("litellm_lens") && *metadata.level() <= tracing::Level::INFO
    }
    fn emit(&self, record: &litellm_tracing::Record) {
        let _ = writeln!(
            std::io::stderr(),
            "{}",
            serde_json::json!({"level": record.metadata.level().as_str(), "message": record.message, "fields": record.fields})
        );
    }
}

fn main() -> Result<(), litellm_lens::Error> {
    if std::env::args().any(|arg| arg == "--version") {
        println!(
            "litellm-lens {} protocol={}",
            std::env::var("LITELLM_RELEASE_TAG").unwrap_or_else(|_| "development".into()),
            litellm_lens::wire::PROTOCOL_VERSION
        );
        return Ok(());
    }
    let _ = litellm_tracing::Logger::new(Diagnostics).install_global();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    let outcome = runtime.block_on(run());
    runtime.shutdown_timeout(Duration::from_secs(10));
    outcome
}

async fn run() -> Result<(), litellm_lens::Error> {
    let config = Config::from_env()?;
    let client = http_client()?;
    let control = Control::new(
        client.clone(),
        config.proxy_url.clone(),
        config.worker_token.clone(),
    );
    let storage = Storage::new(config.storage, client.clone(), config.service_token.clone());
    let state = Arc::new(State::new(storage, config.service_token.clone()));
    let (api, eval_task) = match config.authentication {
        Some(settings) => {
            let authentication = api::authentication(&state, settings).await?;
            let connection = state.storage.config.storage();
            let store = authentication.sessions.0.clone();
            let traces = litellm_traces_clickhouse::evals::EvalTraces::new(
                client.clone(),
                connection.reader().clone(),
            );
            let eval_task = litellm_lens::eval_runtime::start(
                litellm_storage_clickhouse::evals::EvalStore::new(store),
                traces.clone(),
                litellm_lens::eval_judge::GatewayJudge::new(
                    client.clone(),
                    config.proxy_url.clone(),
                    config.eval_judge_api_key.clone(),
                    config.eval_judge_model.clone(),
                ),
            );
            (
                api::router_with_evals(
                    &state,
                    authentication,
                    config.datasets,
                    config.public_url,
                    traces,
                ),
                Some(eval_task),
            )
        }
        None => (lens_server::router(), None),
    };
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    let auth_task = tokio::spawn(auth::refresh_loop(
        state.credentials.clone(),
        client,
        control.url("lens/internal/ingestion-credentials")?,
        config.service_token,
    ));
    let provision_task = tokio::spawn(provision(state.clone()));
    let mut worker = tokio::spawn(Worker::new(control, config.release).serve());
    let (shutdown, stopping) = tokio::sync::oneshot::channel::<()>();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router(state).merge(api))
            .with_graceful_shutdown(async {
                let _ = stopping.await;
            })
            .await
    });
    let outcome = tokio::select! {
        _ = shutdown_signal() => Ok(()),
        _ = &mut worker => Err(litellm_lens::Error::Unavailable),
        result = &mut server => {
            auth_task.abort(); provision_task.abort(); worker.abort();
            if let Some(task) = eval_task { task.abort(); }
            return result.map_err(|_| litellm_lens::Error::Unavailable)?.map_err(Into::into);
        }
    };
    let _ = shutdown.send(());
    auth_task.abort();
    provision_task.abort();
    worker.abort();
    if let Some(task) = eval_task {
        task.abort();
    }
    let _ = worker.await;
    if tokio::time::timeout(Duration::from_secs(10), &mut server)
        .await
        .is_err()
    {
        server.abort();
    }
    outcome
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = signal.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
