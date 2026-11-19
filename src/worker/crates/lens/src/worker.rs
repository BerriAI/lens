use crate::{Error, control::JobClient, pipeline, wire};
use std::{future::Future, time::Duration};

#[derive(Debug, PartialEq, Eq)]
pub enum Execution {
    Finished,
    Interrupted,
}

pub async fn execute(claim: wire::Claim, client: JobClient) -> Result<(), Error> {
    execute_until(claim, client, std::future::pending())
        .await
        .map(|_| ())
}

pub async fn execute_until(
    mut claim: wire::Claim,
    client: JobClient,
    deadline: impl Future<Output = ()>,
) -> Result<Execution, Error> {
    let work = async {
        let sample = client.sample().await?;
        claim.reviews = Some(client.reviews().await?);
        let result = pipeline::analyze(&claim, sample, client.clone()).await?;
        client.finish(&result).await?;
        Ok::<_, Error>(())
    };
    let pulse = async {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            match client.heartbeat().await {
                Ok(_) => {}
                Err(Error::Request(_))
                | Err(Error::Control {
                    status: 429 | 500..=599,
                    ..
                }) => tracing::warn!("Lens heartbeat failed; retrying"),
                Err(error) => return Err::<(), _>(error),
            }
        }
    };
    let outcome = tokio::select! {
        result = work => result,
        result = pulse => result,
        () = deadline => return Ok(Execution::Interrupted),
    };
    match outcome {
        Ok(()) => {}
        Err(Error::JobOwnershipLost) => {
            tracing::info!(job_id = %claim.job.id, category = "ownership_lost", "Lens analysis stopped after losing job ownership");
        }
        Err(error) => {
            tracing::warn!(job_id = %claim.job.id, category = "analysis_failed", "Lens analysis failed; recording the failure");
            failure(&client, &claim.job.id, &error.to_string()).await?;
        }
    }
    Ok(Execution::Finished)
}

async fn failure(client: &JobClient, job_id: &str, message: &str) -> Result<(), Error> {
    let result = wire::Result {
        coverage: wire::Coverage::default(),
        findings: Vec::new(),
        assessments: Vec::new(),
        review_versions: Vec::new(),
        error: message.into(),
    };
    match client.finish(&result).await {
        Ok(_) => Ok(()),
        Err(Error::JobOwnershipLost) => {
            tracing::info!(
                job_id,
                category = "ownership_lost",
                "Lens failure could not be recorded after losing job ownership"
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}
