use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::header::{HeaderName, HeaderValue, SET_COOKIE};
use reqwest::{Method, Response};
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::error::{Error, Result};
use crate::matcher::{render_diff, response_mismatches};
use crate::model::{Fixture, FixtureFailure, ReplayReport, RequestFixture, ResponseFixture};
use crate::normalize::normalize_response;

#[derive(Clone, Debug)]
pub struct Tokens {
    pub admin_token: String,
    pub gateway_secret: String,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub directory: String,
    pub name: String,
    pub request: RequestFixture,
    pub captures: Vec<Capture>,
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub binding: String,
    pub source: CaptureSource,
}

#[derive(Clone, Debug)]
pub enum CaptureSource {
    JsonPointer(String),
    SessionCookie,
}

#[derive(Clone, Debug, Serialize)]
struct GatewayClaims<'a> {
    iss: &'a str,
    aud: &'a str,
    sub: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<i64>,
    exp: i64,
    identity: GatewayIdentity<'a>,
}

#[derive(Clone, Debug, Serialize)]
struct GatewayIdentity<'a> {
    user_role: &'a str,
    user_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<&'a str>,
}

#[derive(Clone, Copy)]
struct IdentityClaims {
    role: &'static str,
    user_id: &'static str,
    team_id: Option<&'static str>,
}

impl Tokens {
    pub fn new(admin_token: impl Into<String>, gateway_secret: impl Into<String>) -> Self {
        Self {
            admin_token: admin_token.into(),
            gateway_secret: gateway_secret.into(),
        }
    }
}

pub async fn record_scenarios(
    base_url: &Url,
    fixtures_dir: &Path,
    scenarios: &[Scenario],
    tokens: &Tokens,
) -> Result<usize> {
    let mut bindings_by_directory: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for scenario in scenarios {
        let bindings = bindings_by_directory
            .entry(scenario.directory.clone())
            .or_default();
        let response = send_request(base_url, &scenario.request, bindings, tokens).await?;
        for capture in &scenario.captures {
            let value = capture_value(capture, &response)?;
            bindings.insert(capture.binding.clone(), value);
        }
        let mut normalized = response;
        normalize_response(&mut normalized, bindings);
        let fixture = Fixture {
            request: scenario.request.clone(),
            response: normalized,
        };
        let path = fixture_path(fixtures_dir, &scenario.directory, &scenario.name);
        write_fixture(&path, &fixture).await?;
    }
    Ok(scenarios.len())
}

pub async fn replay_fixtures(
    base_url: &Url,
    fixtures_dir: &Path,
    tokens: &Tokens,
) -> Result<ReplayReport> {
    let directories = sorted_directories(fixtures_dir).await?;
    let directories = if directories.is_empty() {
        vec![fixtures_dir.to_owned()]
    } else {
        directories
    };
    let mut report = ReplayReport::default();
    for directory in directories {
        let paths = sorted_fixtures(&directory).await?;
        let mut bindings = BTreeMap::new();
        for path in paths {
            let fixture = read_fixture(&path).await?;
            let actual = send_request(base_url, &fixture.request, &bindings, tokens).await?;
            let mismatches = response_mismatches(&fixture.response, &actual, &mut bindings)?;
            if mismatches.is_empty() {
                report.passed += 1;
            } else {
                report.failures.push(FixtureFailure {
                    file_name: path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_owned(),
                    diff: render_diff(&fixture.response, &actual),
                    mismatches,
                });
            }
        }
    }
    Ok(report)
}

async fn send_request(
    base_url: &Url,
    template: &RequestFixture,
    bindings: &BTreeMap<String, String>,
    tokens: &Tokens,
) -> Result<ResponseFixture> {
    let path = expand_text(&template.path, bindings, tokens, base_url)?;
    let url = base_url.join(&path)?;
    let method = Method::from_bytes(template.method.as_bytes())
        .map_err(|_| Error::Unsupported(format!("HTTP method {}", template.method)))?;
    let client = reqwest::Client::new();
    let mut request = client.request(method, url);
    for (name, value) in &template.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| Error::Unsupported(format!("HTTP header name {name}")))?;
        let value = expand_text(value, bindings, tokens, base_url)?;
        let value = HeaderValue::from_str(&value)
            .map_err(|_| Error::Unsupported(format!("HTTP header value for {name}")))?;
        request = request.header(name, value);
    }
    let body = expand_value(&template.body, bindings, tokens, base_url)?;
    if !body.is_null() {
        request = request.json(&body);
    }
    let response = request.send().await?;
    response_fixture(response).await
}

async fn response_fixture(response: Response) -> Result<ResponseFixture> {
    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for name in ["content-type", "content-disposition"] {
        if let Some(value) = response.headers().get(name) {
            headers.insert(
                name.to_owned(),
                Value::String(value.to_str().unwrap_or_default().to_owned()),
            );
        }
    }
    let cookies: Vec<Value> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| Value::String(value.to_str().unwrap_or_default().to_owned()))
        .collect();
    match cookies.as_slice() {
        [] => {}
        [cookie] => {
            headers.insert("set-cookie".to_owned(), cookie.clone());
        }
        _ => {
            headers.insert("set-cookie".to_owned(), Value::Array(cookies));
        }
    }
    let bytes = response.bytes().await?;
    let body = if bytes.is_empty() {
        Value::Null
    } else if headers
        .get("content-type")
        .and_then(Value::as_str)
        .is_some_and(is_json_content_type)
    {
        serde_json::from_slice(&bytes).map_err(|source| Error::Json {
            path: PathBuf::from("<HTTP response>"),
            source,
        })?
    } else {
        Value::String(
            String::from_utf8(bytes.to_vec())
                .map_err(|_| Error::Unsupported("non-UTF-8 HTTP response body".to_owned()))?,
        )
    };
    Ok(ResponseFixture {
        status,
        headers,
        body,
    })
}

fn is_json_content_type(content_type: &str) -> bool {
    content_type.split(';').next().is_some_and(|media_type| {
        media_type.trim() == "application/json" || media_type.trim().ends_with("+json")
    })
}

fn capture_value(capture: &Capture, response: &ResponseFixture) -> Result<String> {
    match &capture.source {
        CaptureSource::JsonPointer(pointer) => response
            .body
            .pointer(pointer)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            })
            .ok_or_else(|| Error::MissingCapture {
                binding: capture.binding.clone(),
                capture_source: pointer.clone(),
            }),
        CaptureSource::SessionCookie => response
            .headers
            .get("set-cookie")
            .and_then(|value| match value {
                Value::String(cookie) => Some(vec![cookie.as_str()]),
                Value::Array(cookies) => {
                    Some(cookies.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                }
                _ => None,
            })
            .and_then(|cookies| {
                cookies.into_iter().find_map(|cookie| {
                    let (name, rest) = cookie.split_once('=')?;
                    (name == "lens_session")
                        .then(|| rest.split(';').next().unwrap_or_default().to_owned())
                })
            })
            .ok_or_else(|| Error::MissingCapture {
                binding: capture.binding.clone(),
                capture_source: "set-cookie lens_session".to_owned(),
            }),
    }
}

fn expand_value(
    value: &Value,
    bindings: &BTreeMap<String, String>,
    tokens: &Tokens,
    base_url: &Url,
) -> Result<Value> {
    match value {
        Value::String(text) => Ok(Value::String(expand_text(
            text, bindings, tokens, base_url,
        )?)),
        Value::Array(values) => Ok(Value::Array(
            values
                .iter()
                .map(|value| expand_value(value, bindings, tokens, base_url))
                .collect::<Result<Vec<_>>>()?,
        )),
        Value::Object(values) => Ok(Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.clone(),
                        expand_value(value, bindings, tokens, base_url)?,
                    ))
                })
                .collect::<Result<_>>()?,
        )),
        scalar => Ok(scalar.clone()),
    }
}

fn expand_text(
    text: &str,
    bindings: &BTreeMap<String, String>,
    tokens: &Tokens,
    base_url: &Url,
) -> Result<String> {
    let mut expanded = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("{{") {
        expanded.push_str(&remaining[..start]);
        let end = remaining[start + 2..]
            .find("}}")
            .map(|end| end + start + 2)
            .ok_or_else(|| Error::Unsupported(format!("unterminated placeholder in {text:?}")))?;
        let name = &remaining[start + 2..end];
        expanded.push_str(&resolve_placeholder(name, bindings, tokens, base_url)?);
        remaining = &remaining[end + 2..];
    }
    expanded.push_str(remaining);
    Ok(expanded)
}

fn resolve_placeholder(
    name: &str,
    bindings: &BTreeMap<String, String>,
    tokens: &Tokens,
    base_url: &Url,
) -> Result<String> {
    match name {
        "admin_token" => Ok(tokens.admin_token.clone()),
        "origin" => Ok(base_url.origin().ascii_serialization()),
        name if name.starts_with("jwt.") => mint_jwt(&name[4..], &tokens.gateway_secret),
        name => bindings
            .get(name)
            .cloned()
            .ok_or_else(|| Error::Unbound(name.to_owned())),
    }
}

fn mint_jwt(case: &str, secret: &str) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    let admin = IdentityClaims {
        role: "proxy_admin",
        user_id: "gateway-admin",
        team_id: Some("alpha"),
    };
    let base_case = match case {
        "integer_strings" | "integral_floats" | "fractional_iat" | "unknown_identity"
        | "unknown_claim" | "wrong_signature" | "wrong_algorithm" | "token_subject" | "future"
        | "maximum_lifetime" => "valid",
        case => case,
    };
    let (identity, subject, issued_at, expires_at, audience, issuer) = match base_case {
        "valid" => (
            admin,
            admin.user_id,
            Some(now),
            now + 30,
            "litellm-lens",
            "litellm",
        ),
        "expired" => (
            admin,
            admin.user_id,
            Some(now - 120),
            now - 90,
            "litellm-lens",
            "litellm",
        ),
        "wrong_audience" => (
            admin,
            admin.user_id,
            Some(now),
            now + 30,
            "litellm",
            "litellm",
        ),
        "long_lifetime" => (
            admin,
            admin.user_id,
            Some(now),
            now + 61,
            "litellm-lens",
            "litellm",
        ),
        "sub_mismatch" => (
            admin,
            "someone-else",
            Some(now),
            now + 30,
            "litellm-lens",
            "litellm",
        ),
        "wrong_issuer" => (
            admin,
            admin.user_id,
            Some(now),
            now + 30,
            "litellm-lens",
            "not-litellm",
        ),
        "missing_claim" => (
            admin,
            admin.user_id,
            None,
            now + 30,
            "litellm-lens",
            "litellm",
        ),
        "other_team" => (
            IdentityClaims {
                role: "internal_user",
                user_id: "beta-user",
                team_id: Some("beta"),
            },
            "beta-user",
            Some(now),
            now + 30,
            "litellm-lens",
            "litellm",
        ),
        "other_team_viewer" => (
            IdentityClaims {
                role: "proxy_admin_viewer",
                user_id: "beta-viewer",
                team_id: Some("beta"),
            },
            "beta-viewer",
            Some(now),
            now + 30,
            "litellm-lens",
            "litellm",
        ),
        "no_team" => (
            IdentityClaims {
                role: "internal_user",
                user_id: "solo-user",
                team_id: None,
            },
            "solo-user",
            Some(now),
            now + 30,
            "litellm-lens",
            "litellm",
        ),
        _ => return Err(Error::Unsupported(format!("JWT case {case}"))),
    };
    let claims = GatewayClaims {
        iss: issuer,
        aud: audience,
        sub: subject,
        iat: issued_at,
        exp: expires_at,
        identity: GatewayIdentity {
            user_role: identity.role,
            user_id: identity.user_id,
            team_id: identity.team_id,
        },
    };
    let mut claims = serde_json::to_value(claims).expect("gateway fixture serializes");
    match case {
        "integer_strings" => {
            claims["iat"] = Value::from(now.to_string());
            claims["exp"] = Value::from((now + 30).to_string());
        }
        "integral_floats" => {
            claims["iat"] = Value::from(now as f64);
            claims["exp"] = Value::from((now + 30) as f64);
        }
        "fractional_iat" => {
            claims["iat"] = Value::from(now as f64 - 0.5);
        }
        "unknown_identity" => {
            claims["identity"]["extra"] = Value::from(true);
        }
        "unknown_claim" => {
            claims["nbf"] = Value::from(now - 1);
        }
        "token_subject" => {
            claims["identity"] = serde_json::json!({"token": "gateway-key", "user_role": "team"});
            claims["sub"] = Value::from("gateway-key");
        }
        "future" => {
            claims["iat"] = Value::from(now + 30);
        }
        "maximum_lifetime" => {
            claims["exp"] = Value::from(now + 60);
        }
        _ => {}
    }
    let algorithm = if case == "wrong_algorithm" {
        Algorithm::HS384
    } else {
        Algorithm::HS256
    };
    let secret = if case == "wrong_signature" {
        "invalid-signature-secret"
    } else {
        secret
    };
    Ok(jsonwebtoken::encode(
        &Header::new(algorithm),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?)
}

fn fixture_path(fixtures_dir: &Path, directory: &str, name: &str) -> PathBuf {
    fixtures_dir.join(directory).join(format!("{name}.json"))
}

async fn write_fixture(path: &Path, fixture: &Fixture) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    let mut contents = serde_json::to_vec_pretty(fixture).expect("fixture serializes");
    contents.push(b'\n');
    tokio::fs::write(path, contents)
        .await
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })
}

async fn read_fixture(path: &Path) -> Result<Fixture> {
    let contents = tokio::fs::read(path).await.map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_slice(&contents).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}

async fn sorted_directories(root: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = tokio::fs::read_dir(root)
        .await
        .map_err(|source| Error::Io {
            path: root.to_owned(),
            source,
        })?;
    let mut directories = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|source| Error::Io {
        path: root.to_owned(),
        source,
    })? {
        if entry
            .file_type()
            .await
            .map_err(|source| Error::Io {
                path: entry.path(),
                source,
            })?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

async fn sorted_fixtures(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .map_err(|source| Error::Io {
            path: directory.to_owned(),
            source,
        })?;
    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|source| Error::Io {
        path: directory.to_owned(),
        source,
    })? {
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::{Capture, CaptureSource, capture_value, is_json_content_type, mint_jwt};
    use crate::model::ResponseFixture;

    struct ExpectedClaims {
        case: &'static str,
        issuer: &'static str,
        audience: &'static str,
        subject: &'static str,
        role: &'static str,
        team_id: Option<&'static str>,
        issued_at_offset: Option<i64>,
        lifetime: i64,
    }

    #[rstest]
    #[case::json("application/json", true)]
    #[case::json_parameters("application/json; charset=utf-8", true)]
    #[case::structured_suffix("application/problem+json", true)]
    #[case::text("text/plain", false)]
    fn classifies_json_media_types(#[case] content_type: &str, #[case] expected: bool) {
        assert_eq!(is_json_content_type(content_type), expected);
    }

    #[rstest]
    #[case::valid(ExpectedClaims { case: "valid", issuer: "litellm", audience: "litellm-lens", subject: "gateway-admin", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: Some(0), lifetime: 30 })]
    #[case::expired(ExpectedClaims { case: "expired", issuer: "litellm", audience: "litellm-lens", subject: "gateway-admin", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: Some(-120), lifetime: 30 })]
    #[case::wrong_audience(ExpectedClaims { case: "wrong_audience", issuer: "litellm", audience: "litellm", subject: "gateway-admin", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: Some(0), lifetime: 30 })]
    #[case::long_lifetime(ExpectedClaims { case: "long_lifetime", issuer: "litellm", audience: "litellm-lens", subject: "gateway-admin", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: Some(0), lifetime: 61 })]
    #[case::subject_mismatch(ExpectedClaims { case: "sub_mismatch", issuer: "litellm", audience: "litellm-lens", subject: "someone-else", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: Some(0), lifetime: 30 })]
    #[case::wrong_issuer(ExpectedClaims { case: "wrong_issuer", issuer: "not-litellm", audience: "litellm-lens", subject: "gateway-admin", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: Some(0), lifetime: 30 })]
    #[case::missing_iat(ExpectedClaims { case: "missing_claim", issuer: "litellm", audience: "litellm-lens", subject: "gateway-admin", role: "proxy_admin", team_id: Some("alpha"), issued_at_offset: None, lifetime: 30 })]
    #[case::other_team(ExpectedClaims { case: "other_team", issuer: "litellm", audience: "litellm-lens", subject: "beta-user", role: "internal_user", team_id: Some("beta"), issued_at_offset: Some(0), lifetime: 30 })]
    #[case::other_team_viewer(ExpectedClaims { case: "other_team_viewer", issuer: "litellm", audience: "litellm-lens", subject: "beta-viewer", role: "proxy_admin_viewer", team_id: Some("beta"), issued_at_offset: Some(0), lifetime: 30 })]
    #[case::no_team(ExpectedClaims { case: "no_team", issuer: "litellm", audience: "litellm-lens", subject: "solo-user", role: "internal_user", team_id: None, issued_at_offset: Some(0), lifetime: 30 })]
    fn mints_jwt_claims_with_relative_offsets(#[case] expected: ExpectedClaims) {
        let before = chrono::Utc::now().timestamp();
        let token = mint_jwt(expected.case, "parity-secret").unwrap();
        let after = chrono::Utc::now().timestamp();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        validation.validate_aud = false;
        let claims = decode::<Value>(
            &token,
            &DecodingKey::from_secret(b"parity-secret"),
            &validation,
        )
        .unwrap()
        .claims;
        let identity = &claims["identity"];

        assert_eq!(claims["iss"], expected.issuer);
        assert_eq!(claims["aud"], expected.audience);
        assert_eq!(claims["sub"], expected.subject);
        assert_eq!(identity["user_role"], expected.role);
        if expected.case == "sub_mismatch" {
            assert_eq!(identity["user_id"], "gateway-admin");
        } else {
            assert_eq!(identity["user_id"], expected.subject);
        }
        assert_eq!(
            identity.get("team_id").and_then(Value::as_str),
            expected.team_id
        );
        if let Some(offset) = expected.issued_at_offset {
            let issued_at = claims["iat"].as_i64().unwrap();
            assert!((before + offset..=after + offset).contains(&issued_at));
            assert_eq!(
                claims["exp"].as_i64().unwrap() - issued_at,
                expected.lifetime
            );
        } else {
            assert!(claims.get("iat").is_none());
            let expires_at = claims["exp"].as_i64().unwrap();
            assert!((before + expected.lifetime..=after + expected.lifetime).contains(&expires_at));
        }
    }

    #[rstest]
    #[case::single_cookie(json!("lens_session=single-value; HttpOnly"), "single-value")]
    #[case::multiple_cookies(json!(["other=other-value; Path=/", "lens_session=multi-value; Path=/"]), "multi-value")]
    fn captures_session_cookie_values(#[case] set_cookie: Value, #[case] expected: &str) {
        let response = ResponseFixture {
            status: 200,
            headers: BTreeMap::from([("set-cookie".to_owned(), set_cookie)]),
            body: Value::Null,
        };
        let capture = Capture {
            binding: "session_cookie".to_owned(),
            source: CaptureSource::SessionCookie,
        };

        assert_eq!(capture_value(&capture, &response).unwrap(), expected);
    }
}
