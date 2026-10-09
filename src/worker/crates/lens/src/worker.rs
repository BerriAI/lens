use crate::{Error, control::JobClient, pipeline, wire};
use std::time::Duration;

pub async fn execute(mut claim: wire::Claim, client: JobClient) -> Result<(), Error> {
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
    let outcome = tokio::select! { result = work => result, result = pulse => result };
    match outcome {
        Ok(()) | Err(Error::Control { status: 409, .. }) => {}
        Err(error) => failure(&client, &error.to_string()).await?,
    }
    Ok(())
}

async fn failure(client: &JobClient, message: &str) -> Result<(), Error> {
    let result = wire::Result {
        coverage: wire::Coverage::default(),
        findings: Vec::new(),
        assessments: Vec::new(),
        review_versions: Vec::new(),
        error: message.into(),
    };
    match client.finish(&result).await {
        Ok(_) | Err(Error::Control { status: 409, .. }) => Ok(()),
        Err(error) => Err(error),
    }
}
