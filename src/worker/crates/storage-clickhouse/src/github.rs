use lens_contract::github::{Authorization, AuthorizationState, Connection};
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

impl GitHubStore {
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
