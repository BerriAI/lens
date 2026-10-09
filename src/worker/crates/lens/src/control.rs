use crate::Error;
use http::Method;
use litellm_http::Client;
use serde::{Serialize, de::DeserializeOwned};
use std::{sync::Arc, time::Duration};
use url::Url;

const MAX_RESPONSE: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct Control {
    client: Client,
    base: Url,
    token: Arc<str>,
    gateway: Option<lens_inference::GatewayIdentity>,
}

impl Control {
    pub fn new(client: Client, mut base: Url, token: String) -> Self {
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        Self {
            client,
            base,
            token: token.into(),
            gateway: None,
        }
    }

    pub fn with_gateway(mut self, gateway: Option<lens_inference::GatewayIdentity>) -> Self {
        self.gateway = gateway;
        self
    }

    pub fn url(&self, path: &str) -> Result<Url, Error> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|_| Error::InvalidRequest)
    }

    pub async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        body: Option<&impl Serialize>,
        timeout: Duration,
    ) -> Result<T, Error> {
        let marker = self
            .gateway
            .as_ref()
            .map(|gateway| {
                gateway.token(
                    &url,
                    lens_inference::GatewayPurpose::Analysis,
                    chrono::Utc::now(),
                )
            })
            .transpose()?
            .flatten();
        let request = self
            .client
            .request(method, url)
            .bearer_auth(&*self.token)
            .timeout(timeout);
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        let request = match marker {
            Some(token) => request.header(lens_inference::GATEWAY_HEADER, token),
            None => request,
        };
        let mut response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            return Err(Error::Control {
                status: status.as_u16(),
                retry_after,
                diagnostic: None,
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                return Err(Error::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            body.extend_from_slice(b"null");
        }
        Ok(serde_json::from_slice(&body)?)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.request(
            Method::GET,
            self.url(path)?,
            None::<&()>,
            Duration::from_secs(180),
        )
        .await
    }

    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        self.request(
            Method::POST,
            self.url(path)?,
            Some(body),
            Duration::from_secs(180),
        )
        .await
    }
}

mod job;
pub use job::{JobBackend, JobClient};
