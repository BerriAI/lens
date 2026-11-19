use crate::error::SignalAlertError;
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{
    aead, hkdf,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Value, json};

pub(super) struct Secret(aead::LessSafeKey);

impl Secret {
    pub fn new(seed: &str) -> Result<Self, SignalAlertError> {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"lens/signal-alerts/v1");
        let extracted = salt.extract(seed.as_bytes());
        let mut key = [0; 32];
        extracted
            .expand(&[b"webhook"], hkdf::HKDF_SHA256)
            .and_then(|material| material.fill(&mut key))
            .map_err(|_| SignalAlertError::Secret)?;
        Ok(Self(aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_256_GCM, &key)
                .map_err(|_| SignalAlertError::Secret)?,
        )))
    }

    pub fn encrypt(&self, value: &str) -> Result<Value, SignalAlertError> {
        let mut nonce = [0; aead::NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| SignalAlertError::Secret)?;
        let mut body = value.as_bytes().to_vec();
        self.0
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(b"signal-alerts/settings"),
                &mut body,
            )
            .map_err(|_| SignalAlertError::Secret)?;
        Ok(json!({"nonce":STANDARD.encode(nonce),"ciphertext":STANDARD.encode(body)}))
    }

    pub fn decrypt(&self, value: &Value) -> Result<String, SignalAlertError> {
        let decode = |key| {
            STANDARD
                .decode(
                    value
                        .get(key)
                        .and_then(Value::as_str)
                        .ok_or(SignalAlertError::Secret)?,
                )
                .map_err(|_| SignalAlertError::Secret)
        };
        let nonce = decode("nonce")?
            .try_into()
            .map_err(|_| SignalAlertError::Secret)?;
        let mut body = decode("ciphertext")?;
        let plaintext = self
            .0
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(b"signal-alerts/settings"),
                &mut body,
            )
            .map_err(|_| SignalAlertError::Secret)?;
        String::from_utf8(plaintext.to_vec()).map_err(|_| SignalAlertError::Secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn credentials_survive_restart_without_plaintext_storage_and_reject_a_different_key() {
        let seed = "example-admin-token-at-least-thirty-two-characters";
        let encrypted = Secret::new(seed)
            .unwrap()
            .encrypt("fake-webhook-secret")
            .unwrap();
        assert!(!encrypted.to_string().contains("fake-webhook-secret"));
        assert_eq!(
            Secret::new(seed).unwrap().decrypt(&encrypted).unwrap(),
            "fake-webhook-secret"
        );
        assert!(
            Secret::new("another-example-admin-token-for-testing")
                .unwrap()
                .decrypt(&encrypted)
                .is_err()
        );
        let changed =
            json!({"nonce": encrypted["nonce"], "ciphertext": STANDARD.encode(b"tampered")});
        assert!(Secret::new(seed).unwrap().decrypt(&changed).is_err());
    }
}
