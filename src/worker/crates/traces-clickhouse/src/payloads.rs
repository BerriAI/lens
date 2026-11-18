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
    Pending,
    Expired,
    Unreadable,
}

impl Unavailable {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Pending => "the full body has not been stored yet",
            Self::Expired => "the full body expired or was deleted",
            Self::Unreadable => "the full body could not be read",
        }
    }

    pub fn note(self) -> String {
        format!("[Lens: {}; showing the stored preview]", self.reason())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Bodies {
    pub messages: Option<String>,
    pub response: Option<String>,
    pub unavailable: Option<Unavailable>,
}

impl Bodies {
    /// Why `stored`, the inline value of `field`, is only a preview of a body that did not resolve.
    pub fn missing(&self, field: Field, stored: &str) -> Option<Unavailable> {
        let resolved = match field {
            Field::Messages => &self.messages,
            Field::Response => &self.response,
        };
        (resolved.is_none() && lens_payloads::is_preview(stored))
            .then_some(self.unavailable.unwrap_or(Unavailable::Pending))
    }
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const PREVIEWED: &str = "[\"…[Lens preview: 9 bytes omitted]\"]";

    #[rstest]
    #[case::resolved(Some("full"), None, PREVIEWED, None)]
    #[case::inline(None, None, "[\"short\"]", None)]
    #[case::inline_despite_a_failure(None, Some(Unavailable::Expired), "[\"short\"]", None)]
    #[case::not_stored_yet(None, None, PREVIEWED, Some(Unavailable::Pending))]
    #[case::expired(
        None,
        Some(Unavailable::Expired),
        PREVIEWED,
        Some(Unavailable::Expired)
    )]
    #[case::unreadable(
        None,
        Some(Unavailable::Unreadable),
        PREVIEWED,
        Some(Unavailable::Unreadable)
    )]
    fn previews_of_unresolved_bodies_say_why(
        #[case] resolved: Option<&str>,
        #[case] unavailable: Option<Unavailable>,
        #[case] stored: &str,
        #[case] expected: Option<Unavailable>,
    ) {
        let bodies = Bodies {
            messages: resolved.map(str::to_owned),
            response: None,
            unavailable,
        };
        assert_eq!(bodies.missing(Field::Messages, stored), expected);
    }

    #[rstest]
    fn a_resolved_field_says_nothing_about_another_field() {
        let bodies = Bodies {
            messages: Some("full".into()),
            ..Bodies::default()
        };
        assert_eq!(
            bodies.missing(Field::Response, PREVIEWED),
            Some(Unavailable::Pending)
        );
    }
}
