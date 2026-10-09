use std::{sync::Arc, time::Duration};

use lens_contract::{
    agent_io::{AgentIo, Completion, InputSource, JsonPointer, PollCompletion, TraceSource},
    eval::TraceAttribute,
};
use reqwest::{
    Method,
    cookie::{CookieStore, Jar},
    header::{AUTHORIZATION, HeaderMap, HeaderValue, ORIGIN},
};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::{
    Error, Result, client, engine,
    model::{Case, CaseResult, TraceRef},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "auth", rename_all = "snake_case", deny_unknown_fields)]
pub enum Connection {
    None {
        base_url_env: String,
    },
    Bearer {
        base_url_env: String,
        token_env: String,
    },
    MoyaiSession {
        base_url_env: String,
        password_env: String,
    },
}

pub struct ResolvedConnection {
    origin: Url,
    authentication: Authentication,
}

enum Authentication {
    None,
    Bearer(String),
    MoyaiSession(String),
}

impl Connection {
    pub fn resolve(
        &self,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<ResolvedConnection> {
        let required = |name: &str| {
            environment(name)
                .filter(|value| !value.trim().is_empty())
                .ok_or(Error::Invalid {
                    message: "Set the local connection environment variable",
                    value: name.to_owned(),
                })
        };
        let base_url_env = match self {
            Self::None { base_url_env }
            | Self::Bearer { base_url_env, .. }
            | Self::MoyaiSession { base_url_env, .. } => base_url_env,
        };
        let endpoint = client::endpoint(&required(base_url_env)?)?;
        let origin =
            Url::parse(&endpoint).map_err(|_| Error::Configuration("Invalid agent origin"))?;
        if origin.path() != "/" {
            return Err(Error::Configuration(
                "Agent connections require an origin without a path",
            ));
        }
        let authentication = match self {
            Self::None { .. } => Authentication::None,
            Self::Bearer { token_env, .. } => Authentication::Bearer(required(token_env)?),
            Self::MoyaiSession { password_env, .. } => {
                Authentication::MoyaiSession(required(password_env)?)
            }
        };
        Ok(ResolvedConnection {
            origin,
            authentication,
        })
    }
}

pub struct Executor {
    io: AgentIo,
    origin: Url,
    http: reqwest::Client,
    headers: HeaderMap,
    secrets: Vec<String>,
    cancel_on_timeout: bool,
}

fn header(value: &str) -> Result<HeaderValue> {
    let mut header = HeaderValue::from_str(value)
        .map_err(|_| Error::Configuration("Agent credentials must be valid HTTP header values"))?;
    header.set_sensitive(true);
    Ok(header)
}

#[derive(Deserialize)]
struct Session {
    authenticated: bool,
    csrf: String,
}
impl Executor {
    pub async fn connect(io: AgentIo, connection: ResolvedConnection) -> Result<Self> {
        io.validate()
            .map_err(|_| Error::Configuration("Invalid saved agent_io contract"))?;
        let jar = Arc::new(Jar::default());
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .cookie_provider(jar.clone())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(40))
            .build()
            .map_err(Error::AgentTransport)?;
        let mut executor = Self {
            io,
            origin: connection.origin,
            http,
            headers: HeaderMap::new(),
            secrets: Vec::new(),
            cancel_on_timeout: false,
        };
        match connection.authentication {
            Authentication::None => {}
            Authentication::Bearer(token) => {
                executor
                    .headers
                    .insert(AUTHORIZATION, header(&format!("Bearer {token}"))?);
                executor.secrets.push(token);
            }
            Authentication::MoyaiSession(password) => {
                executor.validate_session_contract()?;
                executor.cancel_on_timeout = true;
                executor.secrets.push(password.clone());
                executor.headers.insert(
                    ORIGIN,
                    header(&executor.origin.origin().ascii_serialization())?,
                );
                executor
                    .request(
                        Method::POST,
                        "/api/login",
                        Some(&json!({"password": password})),
                        200,
                    )
                    .await?;
                let session: Session = serde_json::from_value(
                    executor
                        .request(Method::GET, "/api/session", None, 200)
                        .await?,
                )
                .map_err(|_| Error::AgentResponse("Agent session response is invalid"))?;
                let has_cookie = jar
                    .cookies(&executor.origin)
                    .and_then(|value| {
                        value.to_str().ok().map(|value| {
                            value.split(';').any(|cookie| {
                                cookie
                                    .trim()
                                    .strip_prefix("workspace_session=")
                                    .is_some_and(|value| !value.is_empty())
                            })
                        })
                    })
                    .unwrap_or(false);
                if !session.authenticated || session.csrf.is_empty() || !has_cookie {
                    return Err(Error::AgentResponse(
                        "Agent authentication did not establish a session",
                    ));
                }
                executor
                    .headers
                    .insert("X-CSRF-Token", header(&session.csrf)?);
                executor.secrets.push(session.csrf);
                if let Some(cookies) = jar
                    .cookies(&executor.origin)
                    .and_then(|value| value.to_str().ok().map(str::to_owned))
                {
                    executor
                        .secrets
                        .extend(cookies.split(';').filter_map(|cookie| {
                            cookie.split_once('=').map(|(_, value)| value.to_owned())
                        }));
                }
            }
        }
        Ok(executor)
    }

    fn validate_session_contract(&self) -> Result<()> {
        if self.io.submit.path != "/api/runs"
            || self.io.submit.json.get("chat_enabled") != Some(&Value::Bool(true))
            || self.io.submit.json.get("mode") != Some(&Value::String("modal".to_owned()))
            || !self.io.input.iter().any(|binding| {
                binding.source == InputSource::TrialRequestId
                    && binding.target.as_str() == "/client_id"
            })
            || self.io.input.iter().any(|binding| {
                matches!(binding.target.as_str(), "/mode" | "/chat_enabled")
                    || (binding.source == InputSource::CaseInput
                        && binding.target.as_str() != "/prompt")
            })
        {
            return Err(Error::Configuration(
                "Session agents require modal chat runs and a stable trial.request_id at /client_id",
            ));
        }
        Ok(())
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        status: u16,
    ) -> Result<Value> {
        let url = self
            .origin
            .join(path)
            .map_err(|_| Error::Configuration("Invalid agent request path"))?;
        if url.origin() != self.origin.origin() {
            return Err(Error::Configuration(
                "Agent requests must remain on the configured origin",
            ));
        }
        let request = self.http.request(method, url).headers(self.headers.clone());
        let response = match body {
            Some(body) => request.json(body),
            None => request,
        }
        .send()
        .await
        .map_err(Error::AgentTransport)?;
        if response.status().as_u16() != status {
            return Err(Error::AgentHttp {
                status: response.status().as_u16(),
            });
        }
        response
            .json()
            .await
            .map_err(|_| Error::AgentResponse("Agent returned invalid JSON"))
    }

    pub async fn run(&self, case: Case, request_id: String, timeout: Duration) -> CaseResult {
        let timeout = match &self.io.completion {
            Completion::Immediate {} => timeout,
            Completion::Poll(poll) => timeout.min(Duration::from_millis(poll.timeout_ms)),
        };
        let mut accepted_id = None;
        match tokio::time::timeout(timeout, self.execute(&case, &request_id, &mut accepted_id))
            .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(Error::AgentTransport(error)))
                if error.is_timeout() && accepted_id.is_some() =>
            {
                self.timed_out(accepted_id.as_deref()).await
            }
            Ok(Err(error)) => engine::failure("AgentError", &error.to_string()),
            Err(_) => self.timed_out(accepted_id.as_deref()).await,
        }
    }

    async fn timed_out(&self, accepted_id: Option<&str>) -> CaseResult {
        let message = match accepted_id.filter(|_| self.cancel_on_timeout) {
            Some(id) => {
                let path = format!("/api/runs/{}/cancel", client::segment(id));
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    self.request(Method::POST, &path, None, 200),
                )
                .await
                {
                    Ok(Ok(_)) => "Agent timed out; cancellation was requested for the accepted job",
                    _ => {
                        "Agent timed out; cancellation failed and the remote job may still be running"
                    }
                }
            }
            None => "Agent timed out; the remote job may still be running",
        };
        engine::failure("AgentTimeoutError", message)
    }

    async fn execute(
        &self,
        case: &Case,
        request_id: &str,
        accepted_id: &mut Option<String>,
    ) -> Result<CaseResult> {
        if !case.followups.is_empty()
            && !self
                .io
                .input
                .iter()
                .any(|binding| binding.source == InputSource::CaseFollowups)
        {
            return Err(Error::Configuration(
                "Dataset followups require an explicit case.followups mapping",
            ));
        }
        let mut body = Value::Object(self.io.submit.json.clone());
        for binding in &self.io.input {
            let input = match binding.source {
                InputSource::CaseInput => Value::String(case.input.clone()),
                InputSource::CaseFollowups => json!(case.followups),
                InputSource::TrialRequestId => Value::String(request_id.to_owned()),
            };
            *binding
                .target
                .resolve_mut(&mut body)
                .ok_or(Error::Configuration("Agent input target is missing"))? = input;
        }
        let accepted = self
            .request(
                Method::POST,
                &self.io.submit.path,
                Some(&body),
                self.io.submit.accepted_status,
            )
            .await?;
        let completed = match &self.io.completion {
            Completion::Immediate {} => accepted.clone(),
            Completion::Poll(poll) => {
                let id = string(
                    &poll.id_pointer,
                    &accepted,
                    "Agent accepted ID must be a nonempty string",
                )?;
                if id.trim().is_empty() || matches!(id, "." | "..") {
                    return Err(Error::AgentResponse(
                        "Agent accepted ID must be a nonempty path segment",
                    ));
                }
                *accepted_id = Some(id.to_owned());
                self.poll(poll, id).await?
            }
        };
        let output = string(
            &self.io.output.pointer,
            &completed,
            "Agent output must be a string",
        )?;
        if self.io.output.require_nonempty && output.trim().is_empty() {
            return Err(Error::AgentResponse("Agent returned an empty output"));
        }
        let trace = self
            .io
            .trace
            .as_ref()
            .map(|mapping| {
                let source = match mapping.source {
                    TraceSource::Accepted => &accepted,
                    TraceSource::Completed => &completed,
                };
                let value = string(
                    &mapping.pointer,
                    source,
                    "Agent trace reference must be a nonempty string",
                )?;
                if value.trim().is_empty() {
                    return Err(Error::AgentResponse(
                        "Agent trace reference must be a nonempty string",
                    ));
                }
                Ok(TraceRef {
                    attribute: match mapping.attribute {
                        TraceAttribute::SessionId => "session.id",
                        TraceAttribute::TraceId => "trace_id",
                    }
                    .to_owned(),
                    value: value.to_owned(),
                })
            })
            .transpose()?;
        Ok(CaseResult {
            trace,
            output: Some(output.to_owned()),
            ..CaseResult::default()
        })
    }

    async fn poll(&self, poll: &PollCompletion, id: &str) -> Result<Value> {
        let path = poll.path_template.replace("{id}", &client::segment(id));
        loop {
            let response = self.request(Method::GET, &path, None, 200).await?;
            let status = string(
                &poll.status_pointer,
                &response,
                "Agent completion status must be a string",
            )?;
            if poll.failure.iter().any(|state| state == status) {
                if let Some(detail) = poll
                    .error_pointer
                    .as_ref()
                    .and_then(|pointer| pointer.resolve(&response))
                    .and_then(Value::as_str)
                    .filter(|detail| !detail.trim().is_empty())
                {
                    let detail = self
                        .secrets
                        .iter()
                        .filter(|value| !value.is_empty())
                        .fold(detail.to_owned(), |detail, secret| {
                            detail.replace(secret, "[redacted]")
                        });
                    return Err(Error::AgentFailure {
                        detail: engine::redact(&detail),
                    });
                }
                return Err(Error::AgentResponse(
                    "Agent execution failed, was cancelled, or was interrupted",
                ));
            }
            if !poll.success.iter().any(|state| state == status) {
                tokio::time::sleep(Duration::from_millis(poll.interval_ms)).await;
                continue;
            }
            let settled = match &poll.require_item {
                None => true,
                Some(item) => {
                    let items = item
                        .array_pointer
                        .resolve(&response)
                        .and_then(Value::as_array)
                        .ok_or(Error::AgentResponse(
                            "Agent completion items must be an array",
                        ))?;
                    items.iter().any(|value| {
                        item.matches.iter().all(|(key, expected)| {
                            value.get(key) == serde_json::to_value(expected).ok().as_ref()
                        })
                    })
                }
            };
            if settled {
                return Ok(response);
            }
            tokio::time::sleep(Duration::from_millis(poll.interval_ms)).await;
        }
    }
}

fn string<'a>(pointer: &JsonPointer, value: &'a Value, message: &'static str) -> Result<&'a str> {
    pointer
        .resolve(value)
        .and_then(Value::as_str)
        .ok_or(Error::AgentResponse(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::collections::BTreeMap;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header as matching_header, method, path},
    };

    #[rstest]
    #[tokio::test]
    async fn request_timeout_after_acceptance_cancels_without_waiting_for_trial_deadline() {
        let server = MockServer::start().await;
        let io = serde_json::from_value(json!({
            "version":1,"connection":"moyai",
            "submit":{"method":"POST","path":"/api/runs","accepted_status":201,"json":{"input":""}},
            "input":[{"source":"case.input","target":"/input"}],
            "completion":{"kind":"poll","id_pointer":"/id","path_template":"/api/runs/{id}","status_pointer":"/status","success":["completed"],"failure":["failed"],"interval_ms":250,"timeout_ms":60000},
            "output":{"pointer":"/summary","require_nonempty":true}
        })).unwrap();
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"accepted"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/runs/accepted"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(1))
                    .set_body_json(json!({"status":"running"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/runs/accepted/cancel"))
            .and(matching_header(
                "Cookie",
                "workspace_session=private-cookie",
            ))
            .and(matching_header("X-CSRF-Token", "private-csrf"))
            .and(matching_header("Origin", server.uri().as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"cancelled"})))
            .expect(1)
            .mount(&server)
            .await;
        let executor = Executor {
            io,
            origin: Url::parse(&server.uri()).unwrap(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(150))
                .build()
                .unwrap(),
            headers: HeaderMap::from_iter([
                (
                    reqwest::header::COOKIE,
                    header("workspace_session=private-cookie").unwrap(),
                ),
                (
                    reqwest::header::HeaderName::from_static("x-csrf-token"),
                    header("private-csrf").unwrap(),
                ),
                (ORIGIN, header(&server.uri()).unwrap()),
            ]),
            secrets: vec![],
            cancel_on_timeout: true,
        };
        let case = Case {
            id: "case".into(),
            input: "prompt".into(),
            followups: vec![],
            meta: BTreeMap::new(),
            expected: String::new(),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            executor.run(case, "request".into(), Duration::from_secs(60)),
        )
        .await
        .unwrap();
        let error = result.error.unwrap();
        assert_eq!(error.r#type, "AgentTimeoutError");
        assert!(error.message.contains("cancellation was requested"));
        assert!(result.output.is_none());
    }
}
