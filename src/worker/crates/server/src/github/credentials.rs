use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    aead, hkdf, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::GitHubError;

pub(super) struct CredentialCipher {
    encryption: aead::LessSafeKey,
    state: hmac::Key,
    verifier: hmac::Key,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    nonce: String,
    ciphertext: String,
}

fn key(secret: &str, purpose: &[u8]) -> Result<[u8; 32], GitHubError> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"lens/github/broker/v1");
    let extracted = salt.extract(secret.as_bytes());
    let mut output = [0; 32];
    extracted
        .expand(&[purpose], hkdf::HKDF_SHA256)
        .and_then(|material| material.fill(&mut output))
        .map_err(|_| GitHubError::Credentials)?;
    Ok(output)
}

fn aad(record: &str) -> aead::Aad<String> {
    aead::Aad::from(format!(
        "lens/github/broker/credentials/aes256gcm/v1\0{record}"
    ))
}

impl CredentialCipher {
    pub(super) fn new(raw_admin_token: &str) -> Result<Self, GitHubError> {
        if raw_admin_token.len() < 32 {
            return Err(GitHubError::Credentials);
        }
        let encryption = aead::UnboundKey::new(
            &aead::AES_256_GCM,
            &key(raw_admin_token, b"credentials/aes256gcm")?,
        )
        .map_err(|_| GitHubError::Credentials)?;
        Ok(Self {
            encryption: aead::LessSafeKey::new(encryption),
            state: hmac::Key::new(hmac::HMAC_SHA256, &key(raw_admin_token, b"callback/state")?),
            verifier: hmac::Key::new(hmac::HMAC_SHA256, &key(raw_admin_token, b"callback/pkce")?),
        })
    }

    pub(super) fn encrypt<T: Serialize>(
        &self,
        value: &T,
        associated_data: &str,
    ) -> Result<Value, GitHubError> {
        let mut nonce = [0; aead::NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| GitHubError::Credentials)?;
        let mut ciphertext = serde_json::to_vec(value).map_err(|_| GitHubError::Credentials)?;
        self.encryption
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aad(associated_data),
                &mut ciphertext,
            )
            .map_err(|_| GitHubError::Credentials)?;
        serde_json::to_value(Envelope {
            version: 1,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
        })
        .map_err(|_| GitHubError::Credentials)
    }

    pub(super) fn decrypt<T: DeserializeOwned>(
        &self,
        value: &Value,
        associated_data: &str,
    ) -> Result<T, GitHubError> {
        let envelope: Envelope =
            serde_json::from_value(value.clone()).map_err(|_| GitHubError::Credentials)?;
        if envelope.version != 1 {
            return Err(GitHubError::Credentials);
        }
        let nonce: [u8; aead::NONCE_LEN] = URL_SAFE_NO_PAD
            .decode(&envelope.nonce)
            .map_err(|_| GitHubError::Credentials)?
            .try_into()
            .map_err(|_| GitHubError::Credentials)?;
        let mut ciphertext = URL_SAFE_NO_PAD
            .decode(&envelope.ciphertext)
            .map_err(|_| GitHubError::Credentials)?;
        let plaintext = self
            .encryption
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aad(associated_data),
                &mut ciphertext,
            )
            .map_err(|_| GitHubError::Credentials)?;
        serde_json::from_slice(plaintext).map_err(|_| GitHubError::Credentials)
    }

    pub(super) fn state(&self, id: &str) -> String {
        let signature = hmac::sign(&self.state, id.as_bytes());
        format!("{id}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
    }

    pub(super) fn authorization_id(&self, state: &str) -> Result<String, GitHubError> {
        let (id, signature) = state.split_once('.').ok_or(GitHubError::Expired)?;
        let parsed = uuid::Uuid::parse_str(id).map_err(|_| GitHubError::Expired)?;
        if parsed.to_string() != id {
            return Err(GitHubError::Expired);
        }
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| GitHubError::Expired)?;
        hmac::verify(&self.state, id.as_bytes(), &signature).map_err(|_| GitHubError::Expired)?;
        Ok(id.to_owned())
    }

    pub(super) fn verifier(&self, id: &str) -> String {
        URL_SAFE_NO_PAD.encode(hmac::sign(&self.verifier, id.as_bytes()).as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use serde_json::json;

    const ADMIN_TOKEN: &str = "test-admin-token-with-at-least-32-bytes";
    const RECORD: &str = "github-broker/connection/team/agent";
    const AUTHORIZATION: &str = "4134c243-8e47-4e37-afda-94a1a0ba68d0";

    #[derive(Deserialize, PartialEq, Serialize)]
    struct Capability {
        token: String,
        repository_id: u64,
    }

    #[fixture]
    fn cipher() -> CredentialCipher {
        CredentialCipher::new(ADMIN_TOKEN).unwrap()
    }

    #[fixture]
    fn capability() -> Capability {
        Capability {
            token: "private-broker-capability".into(),
            repository_id: 42,
        }
    }

    #[rstest]
    fn encrypts_credentials_and_survives_restart(cipher: CredentialCipher, capability: Capability) {
        let envelope = cipher.encrypt(&capability, RECORD).unwrap();
        let encoded = serde_json::to_string(&envelope).unwrap();
        assert!(!encoded.contains(&capability.token));
        let restarted = CredentialCipher::new(ADMIN_TOKEN).unwrap();
        let restored: Capability = restarted.decrypt(&envelope, RECORD).unwrap();
        assert!(restored == capability);
    }

    #[rstest]
    fn fresh_nonce_changes_ciphertext(cipher: CredentialCipher, capability: Capability) {
        let first = cipher.encrypt(&capability, RECORD).unwrap();
        let second = cipher.encrypt(&capability, RECORD).unwrap();
        assert_ne!(first["nonce"], second["nonce"]);
        assert_ne!(first["ciphertext"], second["ciphertext"]);
    }

    #[rstest]
    #[case::wrong_key("a-different-admin-token-with-32-bytes", RECORD)]
    #[case::wrong_record(ADMIN_TOKEN, "github-broker/connection/other/agent")]
    fn rejects_wrong_key_or_record(
        cipher: CredentialCipher,
        capability: Capability,
        #[case] admin_token: &str,
        #[case] record: &str,
    ) {
        let envelope = cipher.encrypt(&capability, RECORD).unwrap();
        let other = CredentialCipher::new(admin_token).unwrap();
        assert!(matches!(
            other.decrypt::<Capability>(&envelope, record),
            Err(GitHubError::Credentials)
        ));
    }

    #[rstest]
    #[case::version("version", json!(2))]
    #[case::algorithm("algorithm", json!("none"))]
    #[case::nonce("nonce", json!(URL_SAFE_NO_PAD.encode([1; aead::NONCE_LEN])))]
    #[case::short_nonce("nonce", json!(URL_SAFE_NO_PAD.encode([1; 3])))]
    #[case::invalid_nonce("nonce", json!("!"))]
    #[case::ciphertext("ciphertext", json!(URL_SAFE_NO_PAD.encode([1; 64])))]
    #[case::invalid_ciphertext("ciphertext", json!("!"))]
    fn rejects_changed_envelope(
        cipher: CredentialCipher,
        capability: Capability,
        #[case] field: &str,
        #[case] changed: Value,
    ) {
        let mut envelope = cipher.encrypt(&capability, RECORD).unwrap();
        envelope[field] = changed;
        assert!(matches!(
            cipher.decrypt::<Capability>(&envelope, RECORD),
            Err(GitHubError::Credentials)
        ));
    }

    #[rstest]
    fn decrypt_requires_matching_value_type(cipher: CredentialCipher, capability: Capability) {
        let envelope = cipher.encrypt(&capability, RECORD).unwrap();
        assert!(matches!(
            cipher.decrypt::<String>(&envelope, RECORD),
            Err(GitHubError::Credentials)
        ));
    }

    #[rstest]
    #[case::empty("")]
    #[case::short("short-admin-token")]
    fn rejects_short_secret(#[case] secret: &str) {
        assert!(matches!(
            CredentialCipher::new(secret),
            Err(GitHubError::Credentials)
        ));
    }

    #[rstest]
    fn signed_state_survives_restart(cipher: CredentialCipher) {
        let state = cipher.state(AUTHORIZATION);
        let restarted = CredentialCipher::new(ADMIN_TOKEN).unwrap();
        assert_eq!(restarted.authorization_id(&state).unwrap(), AUTHORIZATION);
    }

    #[rstest]
    #[case::changed_id("4134c243-8e47-4e37-afda-94a1a0ba68d1")]
    #[case::invalid_id("not-a-uuid")]
    fn rejects_state_with_changed_id(cipher: CredentialCipher, #[case] id: &str) {
        let state = cipher.state(AUTHORIZATION);
        let (_, signature) = state.split_once('.').unwrap();
        assert!(matches!(
            cipher.authorization_id(&format!("{id}.{signature}")),
            Err(GitHubError::Expired)
        ));
    }

    #[rstest]
    #[case::missing_signature("missing")]
    #[case::empty_signature("4134c243-8e47-4e37-afda-94a1a0ba68d0.")]
    #[case::invalid_signature("4134c243-8e47-4e37-afda-94a1a0ba68d0.!?")]
    fn rejects_malformed_state(cipher: CredentialCipher, #[case] state: &str) {
        assert!(matches!(
            cipher.authorization_id(state),
            Err(GitHubError::Expired)
        ));
    }

    #[rstest]
    fn rejects_state_signed_by_another_key(cipher: CredentialCipher) {
        let other = CredentialCipher::new("a-different-admin-token-with-32-bytes").unwrap();
        assert!(matches!(
            cipher.authorization_id(&other.state(AUTHORIZATION)),
            Err(GitHubError::Expired)
        ));
    }

    #[rstest]
    fn verifier_is_secret_and_separate_from_public_state(cipher: CredentialCipher) {
        let verifier = cipher.verifier(AUTHORIZATION);
        let restarted = CredentialCipher::new(ADMIN_TOKEN).unwrap();
        let other = CredentialCipher::new("a-different-admin-token-with-32-bytes").unwrap();
        let state = cipher.state(AUTHORIZATION);
        let (_, signature) = state.split_once('.').unwrap();
        assert_eq!(verifier, restarted.verifier(AUTHORIZATION));
        assert_eq!(verifier.len(), 43);
        assert_ne!(verifier, signature);
        assert_ne!(verifier, other.verifier(AUTHORIZATION));
        assert_ne!(
            verifier,
            cipher.verifier("4134c243-8e47-4e37-afda-94a1a0ba68d1")
        );
        assert!(matches!(
            cipher.authorization_id(&format!("{AUTHORIZATION}.{verifier}")),
            Err(GitHubError::Expired)
        ));
    }
}
