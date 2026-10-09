use lens_contract::github::{
    Authorization, AuthorizationState, BrokerConnection, BrokerHandshake, BrokerHandshakeState,
    Connection,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    Error,
    state::{Change, ClickHouseState, Snapshot},
};

#[derive(Clone)]
pub struct GitHubStore(pub ClickHouseState);

pub struct StoredAuthorization {
    pub authorization: Authorization,
    snapshot: Snapshot,
}

pub struct StoredHandshake {
    pub handshake: BrokerHandshake,
    snapshot: Snapshot,
}

pub struct StoredBrokerConnection {
    pub connection: BrokerConnection,
    snapshot: Snapshot,
}

fn connection_key(scope: &str, agent: &str) -> String {
    format!(
        "github-connection/{:x}/{:x}",
        Sha256::digest(scope),
        Sha256::digest(agent)
    )
}

fn authorization_key(id: &str) -> String {
    format!("github-authorization/{:x}", Sha256::digest(id))
}

fn handshake_key(id: &str) -> String {
    format!("broker-handshake/{:x}", Sha256::digest(id))
}

fn broker_connection_key(id: &str) -> String {
    format!("broker-connection/{:x}", Sha256::digest(id))
}

pub fn remote_credentials_key(scope: &str, agent: &str) -> String {
    format!(
        "github-remote-credentials/{:x}/{:x}",
        Sha256::digest(scope),
        Sha256::digest(agent)
    )
}

fn retired_credentials_key(scope: &str, agent: &str) -> String {
    format!(
        "github-retired-credentials/{:x}/{:x}",
        Sha256::digest(scope),
        Sha256::digest(agent)
    )
}

fn retired_credentials(value: &Value) -> Result<Vec<Value>, Error> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(value.clone()).map_err(|_| Error::InvalidState)
}

fn retire(previous: &Snapshot, credential: &Value) -> Result<Value, Error> {
    let mut credentials = retired_credentials(&previous.value)?;
    if !credential.is_null() && !credentials.contains(credential) {
        if credentials.len() >= 32 {
            return Err(Error::InvalidLimit("retired GitHub credentials"));
        }
        credentials.push(credential.clone());
    }
    Ok(Value::Array(credentials))
}

impl GitHubStore {
    pub async fn create_handshake(
        &self,
        authorization: &Authorization,
        handshake: &BrokerHandshake,
    ) -> Result<(), Error> {
        if authorization.id != handshake.id
            || authorization.agent != handshake.agent
            || handshake.browser_claimed
            || !handshake.browser_hash.is_empty()
            || handshake.state != BrokerHandshakeState::Pending
        {
            return Err(Error::StateConflict);
        }
        let previous_authorization = self.0.read(&authorization_key(&authorization.id)).await?;
        let previous_handshake = self.0.read(&handshake_key(&handshake.id)).await?;
        if !previous_authorization.value.is_null() || !previous_handshake.value.is_null() {
            return Err(Error::StateExists);
        }
        self.0
            .commit(vec![
                Change {
                    previous: previous_authorization,
                    value: serde_json::to_value(authorization).map_err(|_| Error::InvalidState)?,
                },
                Change {
                    previous: previous_handshake,
                    value: serde_json::to_value(handshake).map_err(|_| Error::InvalidState)?,
                },
            ])
            .await
    }

    pub async fn handshake(&self, id: &str) -> Result<Option<StoredHandshake>, Error> {
        let snapshot = self.0.read(&handshake_key(id)).await?;
        if snapshot.value.is_null() {
            return Ok(None);
        }
        let handshake =
            serde_json::from_value(snapshot.value.clone()).map_err(|_| Error::InvalidState)?;
        Ok(Some(StoredHandshake {
            handshake,
            snapshot,
        }))
    }

    pub async fn claim_handshake(
        &self,
        stored: StoredHandshake,
        browser_hash: String,
    ) -> Result<(), Error> {
        if stored.handshake.browser_claimed
            || !stored.handshake.browser_hash.is_empty()
            || stored.handshake.state != BrokerHandshakeState::Pending
            || browser_hash.len() != 64
            || !browser_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(Error::StateConflict);
        }
        let handshake = BrokerHandshake {
            browser_claimed: true,
            browser_hash,
            ..stored.handshake
        };
        self.0
            .commit(vec![Change {
                previous: stored.snapshot,
                value: serde_json::to_value(handshake).map_err(|_| Error::InvalidState)?,
            }])
            .await
    }

    pub async fn select_repository(
        &self,
        stored_authorization: StoredAuthorization,
        stored_handshake: StoredHandshake,
        code_hash: String,
        connection: &Connection,
    ) -> Result<(), Error> {
        if stored_authorization.authorization.id != stored_handshake.handshake.id
            || stored_authorization.authorization.agent != connection.agent
            || stored_handshake.handshake.agent != connection.agent
            || !stored_handshake.handshake.browser_claimed
            || !matches!(
                stored_authorization.authorization.state,
                AuthorizationState::Ready { .. }
            )
            || stored_handshake.handshake.state != BrokerHandshakeState::Pending
        {
            return Err(Error::StateConflict);
        }
        let authorization = Authorization {
            state: AuthorizationState::Connected,
            ..stored_authorization.authorization
        };
        let handshake = BrokerHandshake {
            state: BrokerHandshakeState::Selected {
                code_hash,
                connection: connection.clone(),
            },
            ..stored_handshake.handshake
        };
        self.0
            .commit(vec![
                Change {
                    previous: stored_authorization.snapshot,
                    value: serde_json::to_value(authorization).map_err(|_| Error::InvalidState)?,
                },
                Change {
                    previous: stored_handshake.snapshot,
                    value: serde_json::to_value(handshake).map_err(|_| Error::InvalidState)?,
                },
            ])
            .await
    }

    pub async fn redeem_handshake(
        &self,
        stored: StoredHandshake,
        connection: &BrokerConnection,
    ) -> Result<(), Error> {
        if !matches!(
            &stored.handshake.state,
            BrokerHandshakeState::Selected { connection: selected, .. }
                if selected == &connection.connection
        ) || !stored.handshake.browser_claimed
            || connection.revoked
            || connection.lens_origin != stored.handshake.lens_origin
        {
            return Err(Error::StateConflict);
        }
        let previous = self.0.read(&broker_connection_key(&connection.id)).await?;
        if !previous.value.is_null() {
            return Err(Error::StateExists);
        }
        let handshake = BrokerHandshake {
            state: BrokerHandshakeState::Redeemed,
            ..stored.handshake
        };
        self.0
            .commit(vec![
                Change {
                    previous: stored.snapshot,
                    value: serde_json::to_value(handshake).map_err(|_| Error::InvalidState)?,
                },
                Change {
                    previous,
                    value: serde_json::to_value(connection).map_err(|_| Error::InvalidState)?,
                },
            ])
            .await
    }

    pub async fn broker_connection(
        &self,
        id: &str,
    ) -> Result<Option<StoredBrokerConnection>, Error> {
        let snapshot = self.0.read(&broker_connection_key(id)).await?;
        if snapshot.value.is_null() {
            return Ok(None);
        }
        let connection =
            serde_json::from_value(snapshot.value.clone()).map_err(|_| Error::InvalidState)?;
        Ok(Some(StoredBrokerConnection {
            connection,
            snapshot,
        }))
    }

    pub async fn revoke_broker_connection(
        &self,
        stored: StoredBrokerConnection,
    ) -> Result<(), Error> {
        let connection = BrokerConnection {
            revoked: true,
            ..stored.connection
        };
        self.0
            .commit(vec![Change {
                previous: stored.snapshot,
                value: serde_json::to_value(connection).map_err(|_| Error::InvalidState)?,
            }])
            .await
    }

    pub async fn remote_credentials(
        &self,
        scope: &str,
        agent: &str,
    ) -> Result<Option<Value>, Error> {
        let snapshot = self.0.read(&remote_credentials_key(scope, agent)).await?;
        Ok((!snapshot.value.is_null()).then_some(snapshot.value))
    }

    pub async fn retired_remote_credentials(
        &self,
        scope: &str,
        agent: &str,
    ) -> Result<Vec<Value>, Error> {
        let snapshot = self.0.read(&retired_credentials_key(scope, agent)).await?;
        retired_credentials(&snapshot.value)
    }

    pub async fn remove_retired_remote_credentials(
        &self,
        scope: &str,
        agent: &str,
        credential: &Value,
    ) -> Result<(), Error> {
        let previous = self.0.read(&retired_credentials_key(scope, agent)).await?;
        let credentials = retired_credentials(&previous.value)?;
        if !credentials.contains(credential) {
            return Ok(());
        }
        let value = Value::Array(
            credentials
                .into_iter()
                .filter(|retired| retired != credential)
                .collect(),
        );
        self.0.commit(vec![Change { previous, value }]).await
    }

    pub async fn connect_remote(
        &self,
        stored: StoredAuthorization,
        connection: &Connection,
        encrypted: Value,
    ) -> Result<Option<Value>, Error> {
        if stored.authorization.agent != connection.agent || encrypted.is_null() {
            return Err(Error::StateConflict);
        }
        let scope = &stored.authorization.owner.scope;
        let previous_connection = self
            .0
            .read(&connection_key(scope, &connection.agent))
            .await?;
        let previous_credentials = self
            .0
            .read(&remote_credentials_key(scope, &connection.agent))
            .await?;
        let previous_retired = self
            .0
            .read(&retired_credentials_key(scope, &connection.agent))
            .await?;
        let retired = retire(&previous_retired, &previous_credentials.value)?;
        let replaced_credentials =
            (!previous_credentials.value.is_null()).then(|| previous_credentials.value.clone());
        let authorization = Authorization {
            state: AuthorizationState::Connected,
            ..stored.authorization
        };
        self.0
            .commit(vec![
                Change {
                    previous: stored.snapshot,
                    value: serde_json::to_value(authorization).map_err(|_| Error::InvalidState)?,
                },
                Change {
                    previous: previous_connection,
                    value: serde_json::to_value(connection).map_err(|_| Error::InvalidState)?,
                },
                Change {
                    previous: previous_credentials,
                    value: encrypted,
                },
                Change {
                    previous: previous_retired,
                    value: retired,
                },
            ])
            .await?;
        Ok(replaced_credentials)
    }

    pub async fn disconnect_remote(&self, scope: &str, agent: &str) -> Result<(), Error> {
        let previous_connection = self.0.read(&connection_key(scope, agent)).await?;
        let previous_credentials = self.0.read(&remote_credentials_key(scope, agent)).await?;
        if previous_connection.value.is_null() && previous_credentials.value.is_null() {
            return Ok(());
        }
        let previous_retired = self.0.read(&retired_credentials_key(scope, agent)).await?;
        let retired = retire(&previous_retired, &previous_credentials.value)?;
        self.0
            .commit(vec![
                Change {
                    previous: previous_connection,
                    value: Value::Null,
                },
                Change {
                    previous: previous_credentials,
                    value: Value::Null,
                },
                Change {
                    previous: previous_retired,
                    value: retired,
                },
            ])
            .await
    }

    pub async fn create_authorization(&self, authorization: &Authorization) -> Result<(), Error> {
        let previous = self.0.read(&authorization_key(&authorization.id)).await?;
        if !previous.value.is_null() {
            return Err(Error::StateExists);
        }
        self.0
            .commit(vec![Change {
                previous,
                value: serde_json::to_value(authorization).map_err(|_| Error::InvalidState)?,
            }])
            .await
    }

    pub async fn authorization(&self, id: &str) -> Result<Option<StoredAuthorization>, Error> {
        let snapshot = self.0.read(&authorization_key(id)).await?;
        if snapshot.value.is_null() {
            return Ok(None);
        }
        let authorization =
            serde_json::from_value(snapshot.value.clone()).map_err(|_| Error::InvalidState)?;
        Ok(Some(StoredAuthorization {
            authorization,
            snapshot,
        }))
    }

    pub async fn transition(
        &self,
        stored: StoredAuthorization,
        state: AuthorizationState,
    ) -> Result<(), Error> {
        let authorization = Authorization {
            state,
            ..stored.authorization
        };
        self.0
            .commit(vec![Change {
                previous: stored.snapshot,
                value: serde_json::to_value(authorization).map_err(|_| Error::InvalidState)?,
            }])
            .await
    }

    pub async fn connection(&self, scope: &str, agent: &str) -> Result<Option<Connection>, Error> {
        let snapshot = self.0.read(&connection_key(scope, agent)).await?;
        if snapshot.value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(snapshot.value)
            .map(Some)
            .map_err(|_| Error::InvalidState)
    }

    pub async fn connect(
        &self,
        stored: StoredAuthorization,
        connection: &Connection,
    ) -> Result<(), Error> {
        let previous = self
            .0
            .read(&connection_key(
                &stored.authorization.owner.scope,
                &connection.agent,
            ))
            .await?;
        let authorization = Authorization {
            state: AuthorizationState::Connected,
            ..stored.authorization
        };
        self.0
            .commit(vec![
                Change {
                    previous,
                    value: serde_json::to_value(connection).map_err(|_| Error::InvalidState)?,
                },
                Change {
                    previous: stored.snapshot,
                    value: serde_json::to_value(authorization).map_err(|_| Error::InvalidState)?,
                },
            ])
            .await
    }

    pub async fn disconnect(&self, scope: &str, agent: &str) -> Result<(), Error> {
        let previous = self.0.read(&connection_key(scope, agent)).await?;
        if previous.value.is_null() {
            return Ok(());
        }
        self.0
            .commit(vec![Change {
                previous,
                value: Value::Null,
            }])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use serde_json::json;

    #[fixture]
    fn full_queue() -> Snapshot {
        Snapshot {
            value: Value::Array(
                (0..32)
                    .map(|index| json!({"ciphertext": format!("opaque-{index}")}))
                    .collect(),
            ),
            ..Snapshot::empty("retired-credentials")
        }
    }

    #[rstest]
    fn full_retired_queue_rejects_new_credentials(full_queue: Snapshot) {
        assert!(matches!(
            retire(&full_queue, &json!({"ciphertext": "new"})),
            Err(Error::InvalidLimit(_))
        ));
    }

    #[rstest]
    fn full_retired_queue_keeps_existing_credential_once(full_queue: Snapshot) {
        assert_eq!(
            retire(&full_queue, &json!({"ciphertext": "opaque-7"})).unwrap(),
            full_queue.value
        );
    }
}
