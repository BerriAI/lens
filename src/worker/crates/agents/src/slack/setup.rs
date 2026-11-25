use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, store::Store};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub bot_token: String,
    pub app_token: String,
}

impl Credentials {
    pub async fn validate(&self, client: &reqwest::Client, base: &str) -> Result<(), Error> {
        if !self.bot_token.starts_with("xoxb-")
            || !self.app_token.starts_with("xapp-")
            || self.bot_token.len() > 2048
            || self.app_token.len() > 2048
            || self.bot_token.contains(char::is_whitespace)
            || self.app_token.contains(char::is_whitespace)
        {
            return Err(Error::Invalid(
                "Enter the bot token and app-level token from your Slack app",
            ));
        }
        #[derive(Deserialize)]
        struct Check {
            ok: bool,
        }
        for (method, token, failure) in [
            (
                "auth.test",
                &self.bot_token,
                "Slack rejected the bot token. Install the app to your workspace and copy its Bot User OAuth Token",
            ),
            (
                "apps.connections.open",
                &self.app_token,
                "Slack rejected the app-level token. Enable Socket Mode and generate a token with connections:write",
            ),
        ] {
            let response = client
                .post(format!("{base}/{method}"))
                .bearer_auth(token)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .map_err(|_| Error::Invalid("Could not reach Slack. Try again"))?
                .error_for_status()
                .map_err(|_| Error::Invalid(failure))?
                .json::<Check>()
                .await
                .map_err(|_| Error::Invalid(failure))?;
            if !response.ok {
                return Err(Error::Invalid(failure));
            }
        }
        Ok(())
    }
}

fn cipher(secret: &str) -> Result<aead::LessSafeKey, Error> {
    let key = Sha256::digest(format!("lens-slack-connection\0{secret}"));
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| Error::Runtime)?,
    ))
}

impl Store {
    pub(crate) async fn save_slack_credentials(
        &self,
        input: &Credentials,
        secret: &str,
    ) -> Result<(), Error> {
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| Error::Runtime)?;
        let mut payload = serde_json::to_vec(input)?;
        cipher(secret)?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(b"lens-slack-v1"),
                &mut payload,
            )
            .map_err(|_| Error::Runtime)?;
        let value = [nonce.as_slice(), &payload].concat();
        sqlx::query("INSERT INTO slack_connection VALUES (1,?) ON CONFLICT(id) DO UPDATE SET credentials=excluded.credentials")
            .bind(value).execute(&self.0).await?;
        Ok(())
    }

    pub(crate) async fn slack_credentials(
        &self,
        secret: &str,
    ) -> Result<Option<Credentials>, Error> {
        let Some(mut value) =
            sqlx::query_scalar::<_, Vec<u8>>("SELECT credentials FROM slack_connection WHERE id=1")
                .fetch_optional(&self.0)
                .await?
        else {
            return Ok(None);
        };
        if value.len() < 28 {
            return Err(Error::Runtime);
        }
        let (nonce, payload) = value.split_at_mut(12);
        let nonce = aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| Error::Runtime)?;
        let plain = cipher(secret)?
            .open_in_place(nonce, aead::Aad::from(b"lens-slack-v1"), payload)
            .map_err(|_| Error::Runtime)?;
        Ok(Some(serde_json::from_slice(plain)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    #[fixture]
    fn credentials() -> Credentials {
        Credentials {
            bot_token: "xoxb-fixture-bot-token".into(),
            app_token: "xapp-fixture-app-token".into(),
        }
    }

    #[rstest]
    #[tokio::test]
    async fn saved_connection_survives_restart_without_plaintext_tokens(credentials: Credentials) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents.db");
        let store = Store::open(&path).await.unwrap();
        store
            .save_slack_credentials(&credentials, "fixture-encryption-key")
            .await
            .unwrap();
        let encrypted: Vec<u8> = sqlx::query_scalar("SELECT credentials FROM slack_connection")
            .fetch_one(&store.0)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&encrypted).contains(&credentials.bot_token));
        assert!(store.slack_credentials("different-key").await.is_err());
        store.0.close().await;
        let restored = Store::open(&path).await.unwrap();
        assert_eq!(
            restored
                .slack_credentials("fixture-encryption-key")
                .await
                .unwrap()
                .unwrap()
                .bot_token,
            credentials.bot_token
        );
    }

    #[rstest]
    #[case::valid(true, true, true)]
    #[case::bad_bot(false, true, false)]
    #[case::bad_app(true, false, false)]
    #[tokio::test]
    async fn both_slack_credentials_must_validate(
        credentials: Credentials,
        #[case] bot_ok: bool,
        #[case] app_ok: bool,
        #[case] expected: bool,
    ) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .and(header("authorization", "Bearer xoxb-fixture-bot-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":bot_ok})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apps.connections.open"))
            .and(header("authorization", "Bearer xapp-fixture-app-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok":app_ok,"url":"wss://example.test"})),
            )
            .mount(&server)
            .await;
        assert_eq!(
            credentials
                .validate(&reqwest::Client::new(), &server.uri())
                .await
                .is_ok(),
            expected
        );
    }
}
