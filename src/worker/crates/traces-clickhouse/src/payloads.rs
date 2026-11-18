use std::{collections::BTreeMap, sync::Arc};

use lens_payloads::{Error, Field, ObjectKey, Store};
use litellm_http::Client;
use litellm_storage_clickhouse::{Query, fetch};
use serde::{Deserialize, Serialize};

use crate::Connection;

#[derive(Serialize)]
struct PayloadRefsParams {
    team_id: String,
    api_key: String,
    event_id: String,
}

#[derive(Deserialize)]
struct PayloadRefRow {
    field: String,
    object_key: String,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    bytes: u64,
    sha256: String,
}

struct PayloadRefs;

impl Query for PayloadRefs {
    type Params = PayloadRefsParams;
    type Row = PayloadRefRow;
    const SQL: &'static str = include_str!("../query/payload_refs.sql");
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unavailable {
    Expired,
    Unreadable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Bodies {
    pub messages: Option<String>,
    pub response: Option<String>,
    pub unavailable: Option<Unavailable>,
}

#[derive(Clone)]
pub struct Payloads(Arc<Store>);

impl Payloads {
    pub fn new(store: Arc<Store>) -> Self {
        Self(store)
    }

    pub fn store(&self) -> &Store {
        &self.0
    }

    /// Bodies of one already-authorized spend row. Only keys inside the row's own tenant scope are
    /// fetched, and a body is used only when it matches its recorded length and checksum.
    pub async fn resolve(
        &self,
        client: &Client,
        connection: &Connection,
        team_id: &str,
        api_key: &str,
        event_id: &str,
    ) -> Bodies {
        if event_id.is_empty() {
            return Bodies::default();
        }
        let params = PayloadRefsParams {
            team_id: team_id.to_owned(),
            api_key: api_key.to_owned(),
            event_id: event_id.to_owned(),
        };
        let Ok(rows) = fetch::<PayloadRefs>(client, connection, &params).await else {
            tracing::warn!("payload references unavailable; serving stored previews");
            return Bodies {
                unavailable: Some(Unavailable::Unreadable),
                ..Bodies::default()
            };
        };
        let mut resolved = BTreeMap::new();
        let mut unavailable = None;
        for row in rows {
            let Some(field) = Field::parse(&row.field) else {
                continue;
            };
            let key = ObjectKey::from_stored(row.object_key);
            if !key.is_for(self.0.prefix(), team_id, api_key, event_id, field) {
                tracing::warn!("payload reference outside its tenant scope was ignored");
                unavailable = Some(Unavailable::Unreadable);
                continue;
            }
            match self.0.get(&key, row.bytes, &row.sha256).await {
                Ok(body) => {
                    resolved.insert(field.name(), body);
                }
                Err(error) => {
                    tracing::warn!(%error, "payload body unavailable; serving stored preview");
                    unavailable = Some(match error {
                        Error::Missing => Unavailable::Expired,
                        Error::Configuration(_)
                        | Error::InvalidKey
                        | Error::Request(_)
                        | Error::Integrity => Unavailable::Unreadable,
                    });
                }
            }
        }
        Bodies {
            messages: resolved.remove("messages"),
            response: resolved.remove("response"),
            unavailable,
        }
    }
}
