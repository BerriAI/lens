use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Messages,
    Response,
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Response => "response",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "messages" => Some(Self::Messages),
            "response" => Some(Self::Response),
            _ => None,
        }
    }
}

pub fn scope(team_id: &str, api_key: &str) -> String {
    let digest = Sha256::digest(format!("{team_id}\0{api_key}").as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn valid_event_id(event_id: &str) -> bool {
    (1..=128).contains(&event_id.len())
        && event_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn new(
        prefix: &str,
        event_time: DateTime<Utc>,
        team_id: &str,
        api_key: &str,
        event_id: &str,
        field: Field,
    ) -> Result<Self, Error> {
        if !valid_event_id(event_id) {
            return Err(Error::InvalidKey);
        }
        Ok(Self(format!(
            "{}{}/{}/{event_id}/{}",
            prefix,
            day_prefix(event_time),
            scope(team_id, api_key),
            field.name()
        )))
    }

    pub fn at_millis(
        prefix: &str,
        event_time_ms: i64,
        team_id: &str,
        api_key: &str,
        event_id: &str,
        field: Field,
    ) -> Result<Self, Error> {
        let event_time = DateTime::from_timestamp_millis(event_time_ms)
            .filter(|_| event_time_ms > 0)
            .ok_or(Error::InvalidKey)?;
        Self::new(prefix, event_time, team_id, api_key, event_id, field)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn belongs_to(&self, prefix: &str, team_id: &str, api_key: &str) -> bool {
        self.0
            .strip_prefix(prefix)
            .and_then(|rest| rest.splitn(5, '/').nth(4))
            .is_some_and(|rest| rest.split('/').next() == Some(scope(team_id, api_key).as_str()))
    }

    pub fn from_stored(value: String) -> Self {
        Self(value)
    }
}

pub(crate) fn day_prefix(time: DateTime<Utc>) -> String {
    time.format("v1/%Y/%m/%d").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn key_is_day_partitioned_and_tenant_scoped() {
        let time = Utc.with_ymd_and_hms(2026, 3, 4, 23, 59, 59).unwrap();
        let key =
            ObjectKey::new("lens/", time, "team-a", "key-a", "evt_1", Field::Response).unwrap();

        assert_eq!(
            key.as_str(),
            format!(
                "lens/v1/2026/03/04/{}/evt_1/response",
                scope("team-a", "key-a")
            )
        );
        assert!(key.belongs_to("lens/", "team-a", "key-a"));
        assert!(!key.belongs_to("lens/", "team-b", "key-a"));
        assert!(!key.belongs_to("lens/", "team-a", "key-b"));
    }

    #[rstest]
    #[case::empty("")]
    #[case::slash("a/b")]
    #[case::dot_dot("..")]
    #[case::too_long(&"a".repeat(129))]
    fn invalid_event_ids_cannot_form_keys(#[case] event_id: &str) {
        let time = Utc.with_ymd_and_hms(2026, 3, 4, 0, 0, 0).unwrap();
        assert!(matches!(
            ObjectKey::new("", time, "t", "k", event_id, Field::Messages),
            Err(Error::InvalidKey)
        ));
    }

    #[rstest]
    fn team_and_key_separator_prevents_scope_collisions() {
        assert_ne!(scope("ab", "c"), scope("a", "bc"));
    }
}
