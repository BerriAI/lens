use futures_util::future::BoxFuture;
use litellm_lens::{
    Error,
    config::http_client,
    control::{Control, JobBackend, JobClient},
    wire,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

pub fn client(base: &str, investigation: &str, job: &str, concurrency: usize) -> JobClient {
    JobClient::local(
        Arc::new(Replay {
            control: Control::new(
                http_client().unwrap(),
                base.parse().unwrap(),
                "replay".into(),
            ),
            prefix: format!("replay/{investigation}/{job}"),
        }),
        concurrency,
        Arc::new(Semaphore::new(16)),
    )
}

struct Replay {
    control: Control,
    prefix: String,
}

impl JobBackend for Replay {
    fn sample(&self) -> BoxFuture<'_, Result<wire::Sample, Error>> {
        Box::pin(async { self.control.get(&format!("{}/sample", self.prefix)).await })
    }

    fn reviews(&self) -> BoxFuture<'_, Result<Vec<wire::Review>, Error>> {
        Box::pin(async { self.control.get(&format!("{}/reviews", self.prefix)).await })
    }

    fn content<'a>(
        &'a self,
        execution_id: &'a str,
        cursor: &'a str,
        offset: usize,
    ) -> BoxFuture<'a, Result<wire::ExecutionContent, Error>> {
        Box::pin(async move {
            let mut url = self.control.url(&format!("{}/content", self.prefix))?;
            url.query_pairs_mut()
                .append_pair("execution_id", execution_id)
                .append_pair("cursor", cursor)
                .append_pair("offset", &offset.to_string());
            self.control
                .request(
                    http::Method::GET,
                    url,
                    None::<&()>,
                    Duration::from_secs(180),
                )
                .await
        })
    }

    fn model<'a>(
        &'a self,
        request: &'a wire::ModelRequest,
    ) -> BoxFuture<'a, Result<wire::ModelResult, Error>> {
        Box::pin(async move {
            self.control
                .post(&format!("{}/model", self.prefix), request)
                .await
        })
    }

    fn progress<'a>(&'a self, progress: &'a wire::Progress) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let _: serde_json::Value = self
                .control
                .post(&format!("{}/progress", self.prefix), progress)
                .await?;
            Ok(())
        })
    }

    fn finish<'a>(&'a self, result: &'a wire::Result) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let _: serde_json::Value = self
                .control
                .post(&format!("{}/result", self.prefix), result)
                .await?;
            Ok(())
        })
    }
}
