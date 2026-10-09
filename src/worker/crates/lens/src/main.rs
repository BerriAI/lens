use litellm_lens::{
    State, Storage, api, auth,
    config::{Config, Mode, http_client},
    control::Control,
    provision, router,
    worker::Worker,
};
use std::{io::Write, sync::Arc, time::Duration};

struct Diagnostics;

impl litellm_tracing::Sink for Diagnostics {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        (metadata.target().starts_with("litellm_lens")
            || metadata.target().starts_with("lens_signals"))
            && *metadata.level() <= tracing::Level::INFO
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
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "init") {
        if arguments.len() > 2 {
            return Err(litellm_lens::Error::Configuration(
                "usage: litellm-lens init [directory]",
            ));
        }
        let directory = arguments.get(1).map_or(".", String::as_str);
        let created = litellm_lens::setup::initialize(std::path::Path::new(directory))?;
        println!(
            "{} {directory}/.env",
            if created { "Created" } else { "Kept existing" }
        );
        return Ok(());
    }
    if arguments.len() == 1 && arguments[0] == "--version" {
        println!(
            "litellm-lens {} protocol={}",
            std::env::var("LENS_VERSION")
                .or_else(|_| std::env::var("LITELLM_RELEASE_TAG"))
                .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into()),
            litellm_lens::wire::PROTOCOL_VERSION
        );
        return Ok(());
    }
    let compact = arguments.as_slice() == ["compact-state"];
    if !arguments.is_empty() && !compact {
        return Err(litellm_lens::Error::Configuration(
            "usage: litellm-lens [init [directory] | compact-state | --version]",
        ));
    }
    let _ = litellm_tracing::Logger::new(Diagnostics).install_global();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    let outcome = runtime.block_on(async {
        if compact {
            compact_state().await
        } else {
            run().await
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(10));
    outcome
}

async fn compact_state() -> Result<(), litellm_lens::Error> {
    let config = Config::from_env()?;
    let store = litellm_storage_clickhouse::state::ClickHouseState::new(
        http_client()?,
        config.storage.storage().reader().clone(),
    );
    let mut after = String::new();
    let mut count = 0;
    loop {
        let keys = store.keys("", &after, 100).await?;
        if keys.is_empty() {
            break;
        }
        let references: Vec<_> = keys.iter().map(String::as_str).collect();
        store.compact(&references).await?;
        count += keys.len();
        after = keys.last().cloned().unwrap_or_default();
    }
    println!("Compacted {count} state records; current values and revision fences retained");
    Ok(())
}

async fn run() -> Result<(), litellm_lens::Error> {
    let config = Config::from_env()?;
    let client = http_client()?;
    let storage = Storage::new(config.storage, client.clone(), config.query_secret.clone());
    let state = Arc::new(match &config.mode {
        Mode::Standalone => match config.gateway_service_token {
            Some(token) => State::connected(storage, token),
            None => State::standalone(storage),
        },
        Mode::Gateway(gateway) => State::new(storage, gateway.service_token.clone()),
    });
    let judge_base = match &config.mode {
        Mode::Gateway(gateway) => gateway.proxy_url.clone(),
        Mode::Standalone => config.public_url.clone(),
    };
    let evals = api::EvalConfig {
        judge: litellm_lens::eval_judge::GatewayJudge::new(
            client.clone(),
            judge_base,
            config.eval_judge_api_key.clone(),
            config.eval_judge_model.clone(),
        )
        .with_gateway(config.gateway_inference.clone()),
        public_url: config.public_url.to_string(),
    };
    let application = match config.authentication {
        Some(settings) => {
            let application = api::initialize(
                &state,
                settings,
                config.datasets,
                config.traces,
                matches!(config.mode, Mode::Standalone),
                evals,
            )
            .await?;
            let application = if matches!(config.mode, Mode::Standalone) {
                application
                    .with_local(
                        state.clone(),
                        config.analysis_models,
                        config.evaluation_models,
                        &config.query_secret,
                        config.gateway_inference,
                    )
                    .await?
                    .with_evaluations()?
            } else {
                application
            };
            Some(application.with_service(state.clone(), config.ingestion_url, config.release))
        }
        None => None,
    };
    let eval_task = application.as_ref().map(|app| app.evals.start());
    let mut tasks = tokio::task::JoinSet::new();
    match config.mode {
        Mode::Standalone => {
            let credentials = application
                .as_ref()
                .ok_or(litellm_lens::Error::Configuration("LENS_ADMIN_TOKEN"))?
                .credentials
                .clone();
            if !credentials.synchronize().await? {
                return Err(litellm_lens::Error::Unavailable);
            }
            tasks.spawn(credentials.serve());
            let worker = application
                .as_ref()
                .and_then(|app| app.local_worker.clone())
                .ok_or(litellm_lens::Error::Unavailable)?;
            tasks.spawn(worker.serve());
            let signals = application
                .as_ref()
                .and_then(|app| app.signals_worker.clone())
                .ok_or(litellm_lens::Error::Unavailable)?;
            tasks.spawn(signals.serve());
        }
        Mode::Gateway(gateway) => {
            let control = Control::new(client.clone(), gateway.proxy_url, gateway.worker_token);
            tasks.spawn(auth::refresh_loop(
                state.credentials.clone(),
                client,
                control.url("lens/internal/ingestion-credentials")?,
                gateway.service_token,
            ));
            tasks.spawn(Worker::new(control, gateway.release).serve());
        }
    }
    tasks.spawn(provision(state.clone()));
    let routes = router(state).merge(application.map(|app| app.router).unwrap_or_default());
    let routes = if let Some(directory) = config.ui_directory {
        if !std::fs::metadata(directory.join("index.html"))?.is_file() {
            return Err(litellm_lens::Error::Configuration(
                "LENS_UI_DIRECTORY must contain the built UI index.html",
            ));
        }
        routes.merge(lens_server::ui::router(directory))
    } else {
        routes
    };
    let routes = routes.layer(axum::middleware::from_fn(
        lens_server::routing::redirect_trailing_slash,
    ));
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    let (shutdown, stopping) = tokio::sync::oneshot::channel::<()>();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, routes)
            .with_graceful_shutdown(async {
                let _ = stopping.await;
            })
            .await
    });
    let outcome = tokio::select! {
        _ = shutdown_signal() => Ok(()),
        _ = tasks.join_next() => Err(litellm_lens::Error::Unavailable),
        result = &mut server => {
            tasks.abort_all();
            return result.map_err(|_| litellm_lens::Error::Unavailable)?.map_err(Into::into);
        }
    };
    let _ = shutdown.send(());
    tasks.abort_all();
    if let Some(task) = eval_task {
        task.abort();
    }
    while tasks.join_next().await.is_some() {}
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
