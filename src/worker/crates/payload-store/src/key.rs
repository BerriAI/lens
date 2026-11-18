use chrono::{DateTime, NaiveDate, Utc};
use sha2::{Digest, Sha256};

use crate::Error;

pub(crate) const NAMESPACE: &str = "lens-payloads/v1/";
const SCOPE_HEX_LEN: usize = 32;

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
            "{prefix}{NAMESPACE}{}/{}/{event_id}/{}",
            day_path(event_time.date_naive()),
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

    pub fn is_for(
        &self,
        prefix: &str,
        team_id: &str,
        api_key: &str,
        event_id: &str,
        field: Field,
    ) -> bool {
        Generated::parse(prefix, &self.0).is_some_and(|key| {
            key.scope == scope(team_id, api_key) && key.event_id == event_id && key.field == field
        })
    }

    pub fn from_stored(value: String) -> Self {
        Self(value)
    }
}

pub(crate) fn day_path(day: NaiveDate) -> String {
    day.format("%Y/%m/%d").to_string()
}

pub(crate) struct Generated<'a> {
    pub(crate) day: NaiveDate,
    scope: &'a str,
    event_id: &'a str,
    field: Field,
}

impl<'a> Generated<'a> {
    pub(crate) fn parse(prefix: &str, key: &'a str) -> Option<Self> {
        let rest = key.strip_prefix(prefix)?.strip_prefix(NAMESPACE)?;
        let parts: Vec<&str> = rest.split('/').collect();
        let [year, month, date, scope, event_id, field] = parts[..] else {
            return None;
        };
        let day = NaiveDate::parse_from_str(&format!("{year}/{month}/{date}"), "%Y/%m/%d").ok()?;
        let canonical = day_path(day) == format!("{year}/{month}/{date}")
            && scope.len() == SCOPE_HEX_LEN
            && scope.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            && valid_event_id(event_id);
        let field = Field::parse(field)?;
        canonical.then_some(Self {
            day,
            scope,
            event_id,
            field,
        })
    }
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
                "lens/lens-payloads/v1/2026/03/04/{}/evt_1/response",
                scope("team-a", "key-a")
            )
        );
    }

    #[rstest]
    #[case::own_object("lens/", "team-a", "key-a", "evt_1", Field::Response, true)]
    #[case::other_team("lens/", "team-b", "key-a", "evt_1", Field::Response, false)]
    #[case::other_key("lens/", "team-a", "key-b", "evt_1", Field::Response, false)]
    #[case::other_event("lens/", "team-a", "key-a", "evt_2", Field::Response, false)]
    #[case::other_field("lens/", "team-a", "key-a", "evt_1", Field::Messages, false)]
    #[case::other_prefix("", "team-a", "key-a", "evt_1", Field::Response, false)]
    fn stored_key_is_read_only_for_its_own_event_and_tenant(
        #[case] prefix: &str,
        #[case] team_id: &str,
        #[case] api_key: &str,
        #[case] event_id: &str,
        #[case] field: Field,
        #[case] expected: bool,
    ) {
        let time = Utc.with_ymd_and_hms(2026, 3, 4, 0, 0, 0).unwrap();
        let key =
            ObjectKey::new("lens/", time, "team-a", "key-a", "evt_1", Field::Response).unwrap();
        assert_eq!(
            key.is_for(prefix, team_id, api_key, event_id, field),
            expected
        );
    }

    #[rstest]
    fn generated_keys_parse_back_to_their_day() {
        let time = Utc.with_ymd_and_hms(2026, 3, 4, 23, 59, 59).unwrap();
        let key = ObjectKey::new("p/", time, "t", "k", "evt", Field::Messages).unwrap();
        assert_eq!(
            Generated::parse("p/", key.as_str()).map(|key| key.day),
            NaiveDate::from_ymd_opt(2026, 3, 4)
        );
    }

    #[rstest]
    #[case::other_namespace("p/v1/2026/03/04/{scope}/evt/messages")]
    #[case::outside_prefix("q/lens-payloads/v1/2026/03/04/{scope}/evt/messages")]
    #[case::day_folder_only("p/lens-payloads/v1/2026/03/04/")]
    #[case::unpadded_month("p/lens-payloads/v1/2026/3/04/{scope}/evt/messages")]
    #[case::impossible_date("p/lens-payloads/v1/2026/02/30/{scope}/evt/messages")]
    #[case::short_scope("p/lens-payloads/v1/2026/03/04/abc/evt/messages")]
    #[case::uppercase_scope("p/lens-payloads/v1/2026/03/04/{upper}/evt/messages")]
    #[case::unknown_field("p/lens-payloads/v1/2026/03/04/{scope}/evt/metadata")]
    #[case::extra_segment("p/lens-payloads/v1/2026/03/04/{scope}/evt/messages/copy")]
    #[case::invalid_event("p/lens-payloads/v1/2026/03/04/{scope}/e.v.t/messages")]
    fn keys_lens_did_not_generate_are_not_recognized(#[case] template: &str) {
        let key = template
            .replace("{scope}", &scope("t", "k"))
            .replace("{upper}", &scope("t", "k").to_uppercase());
        assert!(Generated::parse("p/", &key).is_none());
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
