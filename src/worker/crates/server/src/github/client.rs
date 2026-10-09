use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use lens_contract::github::{Connection, Repository};
use reqwest::Method;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use super::GitHubError;

pub struct GitHubApp {
    pub slug: String,
    pub public_url: Url,
    client_id: String,
    client_secret: String,
    key: EncodingKey,
    http: reqwest::Client,
    api: Url,
    web: Url,
}

#[derive(Deserialize)]
struct User {
    id: u64,
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
}

#[derive(Deserialize)]
struct Installations {
    installations: Vec<Installation>,
}

#[derive(Default, Deserialize)]
struct Permissions {
    #[serde(default)]
    admin: bool,
    #[serde(default)]
    maintain: bool,
    #[serde(default)]
    push: bool,
}

#[derive(Deserialize)]
struct GitHubRepository {
    id: u64,
    full_name: String,
    default_branch: String,
    #[serde(default)]
    permissions: Permissions,
}

#[derive(Deserialize)]
struct Repositories {
    repositories: Vec<GitHubRepository>,
}

#[derive(Deserialize)]
struct AccessToken {
    access_token: String,
}

#[derive(Deserialize)]
struct InstallationToken {
    token: String,
}

impl GitHubApp {
    pub fn new(
        slug: String,
        client_id: String,
        client_secret: String,
        pem: &str,
        public_url: Url,
    ) -> Result<Self, GitHubError> {
        if !matches!(public_url.scheme(), "http" | "https")
            || public_url.host_str().is_none()
            || !public_url.username().is_empty()
            || public_url.password().is_some()
            || public_url.query().is_some()
            || public_url.fragment().is_some()
        {
            return Err(GitHubError::Configuration("LENS_PUBLIC_URL"));
        }
        if slug.is_empty()
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(GitHubError::Configuration("LENS_GITHUB_APP_SLUG"));
        }
        if client_id.is_empty() || client_secret.is_empty() {
            return Err(GitHubError::Configuration(
                "LENS_GITHUB_CLIENT_ID and LENS_GITHUB_CLIENT_SECRET",
            ));
        }
        let key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|_| GitHubError::Configuration("LENS_GITHUB_PRIVATE_KEY"))?;
        let http = reqwest::Client::builder()
            .user_agent("lens-github-app")
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(GitHubError::Transport)?;
        Ok(Self {
            slug,
            client_id,
            client_secret,
            key,
            public_url,
            http,
            api: Url::parse("https://api.github.com/").expect("fixed URL"),
            web: Url::parse("https://github.com/").expect("fixed URL"),
        })
    }

    fn signed(&self, purpose: &str, id: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.client_secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(purpose.as_bytes());
        mac.update(b"\0");
        mac.update(id.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    pub(super) fn state(&self, id: &str) -> String {
        format!("{id}.{}", self.signed("state", id))
    }

    pub(super) fn authorization_id(&self, state: &str) -> Result<String, GitHubError> {
        let (id, signature) = state.split_once('.').ok_or(GitHubError::Expired)?;
        uuid::Uuid::parse_str(id).map_err(|_| GitHubError::Expired)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| GitHubError::Expired)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.client_secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(b"state\0");
        mac.update(id.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| GitHubError::Expired)?;
        Ok(id.into())
    }

    fn callback_url(&self) -> String {
        self.public_url
            .join("/lens/github/callback")
            .expect("fixed callback path")
            .into()
    }

    pub(super) fn authorization_url(&self, id: &str, install: bool) -> String {
        if install {
            let mut url = self
                .web
                .join(&format!("apps/{}/installations/new", self.slug))
                .expect("validated slug");
            url.query_pairs_mut().append_pair("state", &self.state(id));
            return url.into();
        }
        let verifier = self.signed("pkce", id);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier));
        let mut url = self.web.join("login/oauth/authorize").expect("fixed path");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.callback_url())
            .append_pair("state", &self.state(id))
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        url.into()
    }

    pub(super) async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> Result<T, GitHubError> {
        let url = self
            .api
            .join(path.trim_start_matches('/'))
            .map_err(|_| GitHubError::Invalid("Invalid GitHub API path"))?;
        if url.origin() != self.api.origin() {
            return Err(GitHubError::Invalid("Invalid GitHub API path"));
        }
        let request = self
            .http
            .request(method, url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        let response = request.send().await.map_err(GitHubError::Transport)?;
        if !response.status().is_success() {
            return Err(GitHubError::Upstream {
                status: response.status().as_u16(),
            });
        }
        response.json().await.map_err(GitHubError::Decode)
    }

    pub(super) async fn repositories(
        &self,
        id: &str,
        code: &str,
    ) -> Result<Vec<Repository>, GitHubError> {
        let response = self
            .http
            .post(
                self.web
                    .join("login/oauth/access_token")
                    .expect("fixed path"),
            )
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", &self.callback_url()),
                ("code_verifier", &self.signed("pkce", id)),
            ])
            .send()
            .await
            .map_err(GitHubError::Transport)?;
        if !response.status().is_success() {
            return Err(GitHubError::Upstream {
                status: response.status().as_u16(),
            });
        }
        let token: AccessToken = response.json().await.map_err(GitHubError::Decode)?;
        let user: User = self
            .request(Method::GET, "/user", &token.access_token, None)
            .await?;
        if user.id == 0 {
            return Err(GitHubError::Forbidden);
        }
        let mut result = Vec::new();
        for page in 1..=20 {
            let installations: Installations = self
                .request(
                    Method::GET,
                    &format!("/user/installations?per_page=100&page={page}"),
                    &token.access_token,
                    None,
                )
                .await?;
            let last = installations.installations.len() < 100;
            for installation in installations.installations {
                for page in 1..=100 {
                    let repos: Repositories = self
                        .request(
                            Method::GET,
                            &format!(
                                "/user/installations/{}/repositories?per_page=100&page={page}",
                                installation.id
                            ),
                            &token.access_token,
                            None,
                        )
                        .await?;
                    let last = repos.repositories.len() < 100;
                    result.extend(
                        repos
                            .repositories
                            .into_iter()
                            .filter(|repo| {
                                repo.permissions.admin
                                    || repo.permissions.maintain
                                    || repo.permissions.push
                            })
                            .map(|repo| Repository {
                                id: repo.id,
                                full_name: repo.full_name,
                                default_branch: repo.default_branch,
                                installation_id: installation.id,
                            }),
                    );
                    if result.len() > 10_000 {
                        return Err(GitHubError::Invalid(
                            "Limit the GitHub App installation to the repositories you want to connect",
                        ));
                    }
                    if last {
                        break;
                    }
                    if page == 100 {
                        return Err(GitHubError::Invalid(
                            "The GitHub repository list is too large. Limit the App installation scope",
                        ));
                    }
                }
            }
            if last {
                result.sort_by(|a, b| a.full_name.cmp(&b.full_name));
                return Ok(result);
            }
        }
        Err(GitHubError::Invalid(
            "The GitHub installation list is too large",
        ))
    }

    pub(super) async fn installation_token(
        &self,
        connection: &Connection,
    ) -> Result<String, GitHubError> {
        #[derive(Serialize)]
        struct Claims<'a> {
            iat: i64,
            exp: i64,
            iss: &'a str,
        }
        let now = Utc::now().timestamp();
        let jwt = jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &Claims {
                iat: now - 60,
                exp: now + 540,
                iss: &self.client_id,
            },
            &self.key,
        )
        .map_err(GitHubError::Signing)?;
        let token: InstallationToken = self
            .request(
                Method::POST,
                &format!(
                    "/app/installations/{}/access_tokens",
                    connection.installation_id
                ),
                &jwt,
                Some(json!({"repository_ids":[connection.repository_id]})),
            )
            .await?;
        Ok(token.token)
    }

    pub(super) async fn verify_connection(
        &self,
        connection: &Connection,
    ) -> Result<Repository, GitHubError> {
        let token = self.installation_token(connection).await?;
        self.repository(connection, &token).await
    }

    pub(super) async fn repository(
        &self,
        connection: &Connection,
        token: &str,
    ) -> Result<Repository, GitHubError> {
        let repos: Repositories = self
            .request(
                Method::GET,
                "/installation/repositories?per_page=100",
                token,
                None,
            )
            .await?;
        repos
            .repositories
            .into_iter()
            .find(|repo| repo.id == connection.repository_id)
            .map(|repo| Repository {
                id: repo.id,
                full_name: repo.full_name,
                default_branch: repo.default_branch,
                installation_id: connection.installation_id,
            })
            .ok_or(GitHubError::Forbidden)
    }

    #[cfg(test)]
    pub(super) fn with_endpoints(mut self, url: &str) -> Self {
        self.api = Url::parse(&format!("{url}/")).unwrap();
        self.web = self.api.clone();
        self
    }
}
