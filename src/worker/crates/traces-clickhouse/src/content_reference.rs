use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const MAX_CONTENT_OBJECT_BYTES: u64 = 8 * 1024 * 1024;
const PAGE_CHARS: usize = 8000;

pub struct Owner<'a> {
    pub team_id: &'a str,
    pub api_key: &'a str,
    pub request_id: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContentReference {
    Stored {
        object_key: String,
        bytes: u64,
        sha256: String,
    },
    Unavailable(String),
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Signed {
    Stored {
        v: u8,
        object_key: String,
        bytes: u64,
        sha256: String,
        mac: String,
    },
    Unavailable {
        v: u8,
        reason: String,
        mac: String,
    },
}

pub fn signing_key(service_token: &str) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(service_token.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(b"litellm-lens-content");
    mac.finalize().into_bytes().into()
}

fn framed(fields: &[&str]) -> Vec<u8> {
    fields
        .iter()
        .flat_map(|field| {
            format!("{}:", field.len())
                .into_bytes()
                .into_iter()
                .chain(field.bytes())
        })
        .collect()
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn signer(key: &[u8], owner: &Owner<'_>, fields: [&str; 5]) -> Hmac<Sha256> {
    let mut signer = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    signer.update(&framed(&[
        "litellm-lens-content",
        "1",
        owner.team_id,
        owner.api_key,
        owner.request_id,
        fields[0],
        fields[1],
        fields[2],
        fields[3],
        fields[4],
    ]));
    signer
}

fn authentic(key: &[u8], owner: &Owner<'_>, fields: [&str; 5], mac: &str) -> bool {
    decode_hex(mac)
        .is_some_and(|expected| signer(key, owner, fields).verify_slice(&expected).is_ok())
}

pub fn stored_reference(
    key: &[u8],
    owner: &Owner<'_>,
    object_key: &str,
    bytes: u64,
    sha256: &str,
) -> String {
    let mac = signer(
        key,
        owner,
        ["stored", "", object_key, &bytes.to_string(), sha256],
    );
    serde_json::json!({
        "v": 1,
        "state": "stored",
        "object_key": object_key,
        "bytes": bytes,
        "sha256": sha256,
        "mac": format!("{:x}", mac.finalize().into_bytes()),
    })
    .to_string()
}

pub fn verify(key: &[u8], owner: &Owner<'_>, raw: &str) -> Option<ContentReference> {
    match serde_json::from_str::<Signed>(raw).ok()? {
        Signed::Stored {
            v: 1,
            object_key,
            bytes,
            sha256,
            mac,
        } if authentic(
            key,
            owner,
            ["stored", "", &object_key, &bytes.to_string(), &sha256],
            &mac,
        ) && bytes <= MAX_CONTENT_OBJECT_BYTES =>
        {
            Some(ContentReference::Stored {
                object_key,
                bytes,
                sha256,
            })
        }
        Signed::Unavailable { v: 1, reason, mac }
            if authentic(key, owner, ["unavailable", &reason, "", "", ""], &mac) =>
        {
            Some(ContentReference::Unavailable(reason))
        }
        _ => None,
    }
}

pub fn intact(body: &[u8], bytes: u64, sha256: &str) -> bool {
    u64::try_from(body.len()).is_ok_and(|length| length == bytes)
        && format!("{:x}", Sha256::digest(body)) == sha256
}

fn excerpt(value: &str, budget: usize) -> String {
    let length = value.chars().count();
    if length <= budget {
        return value.to_owned();
    }
    let head = budget / 3;
    let start: String = value.chars().take(head).collect();
    let end: String = value.chars().skip(length - (budget - head)).collect();
    format!("{start}\n[... content omitted ...]\n{end}")
}

pub fn page(input: &str, output: &str, error: &str, offset: u32) -> (String, u8) {
    let full = format!("Input: {input}\nOutput: {output}\nError: {error}");
    if offset == 0 {
        return (full, 0);
    }
    let length = full.chars().count();
    let start =
        usize::try_from(offset).map_or(usize::MAX, |offset| offset.saturating_sub(1).max(1));
    let truncated = u8::from(length >= start.saturating_add(PAGE_CHARS));
    if offset == 1 && length > PAGE_CHARS {
        return (
            format!(
                "Input: {}\nOutput: {}\nError: {}",
                excerpt(input, 2000),
                excerpt(output, 5000),
                excerpt(error, 500)
            ),
            truncated,
        );
    }
    (
        full.chars().skip(start - 1).take(PAGE_CHARS).collect(),
        truncated,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const TOKEN: &str = "test-service-token-with-32-characters";
    const OWNER: Owner<'static> = Owner {
        team_id: "team-a",
        api_key: "key-a",
        request_id: "request-a",
    };

    fn sign(owner: &Owner<'_>, fields: [&str; 5]) -> String {
        format!(
            "{:x}",
            signer(&signing_key(TOKEN), owner, fields)
                .finalize()
                .into_bytes()
        )
    }

    fn stored(owner: &Owner<'_>, bytes: u64) -> String {
        stored_reference(&signing_key(TOKEN), owner, "logs/object.json", bytes, "ab")
    }

    #[test]
    fn signed_references_verify_only_for_their_owner() {
        let raw = stored(&OWNER, 10);
        assert_eq!(
            verify(&signing_key(TOKEN), &OWNER, &raw),
            Some(ContentReference::Stored {
                object_key: "logs/object.json".into(),
                bytes: 10,
                sha256: "ab".into()
            })
        );
        for other in [
            Owner {
                team_id: "team-b",
                ..OWNER
            },
            Owner {
                api_key: "key-b",
                ..OWNER
            },
            Owner {
                request_id: "request-b",
                ..OWNER
            },
        ] {
            assert_eq!(verify(&signing_key(TOKEN), &other, &raw), None);
        }
        assert_eq!(
            verify(
                &signing_key("another-token-with-32-characters!!"),
                &OWNER,
                &raw
            ),
            None
        );
    }

    #[rstest]
    #[case::tampered_key(r#""object_key":"logs/other.json""#)]
    #[case::tampered_size(r#""bytes":11"#)]
    fn tampered_references_are_ignored(#[case] replacement: &str) {
        let raw = stored(&OWNER, 10);
        let field = replacement.split(':').next().unwrap();
        let start = raw.find(field).unwrap();
        let end = start + raw[start..].find(',').unwrap();
        let tampered = format!("{}{}{}", &raw[..start], replacement, &raw[end..]);
        assert_eq!(verify(&signing_key(TOKEN), &OWNER, &tampered), None);
    }

    #[test]
    fn unavailable_states_are_signed_and_oversized_references_rejected() {
        let mac = sign(&OWNER, ["unavailable", "upload_failed", "", "", ""]);
        let raw = serde_json::json!({
            "v": 1, "state": "unavailable", "reason": "upload_failed", "mac": mac,
        })
        .to_string();
        assert_eq!(
            verify(&signing_key(TOKEN), &OWNER, &raw),
            Some(ContentReference::Unavailable("upload_failed".into()))
        );
        let forged = serde_json::json!({
            "v": 1, "state": "unavailable", "reason": "too_large", "mac": mac,
        })
        .to_string();
        assert_eq!(verify(&signing_key(TOKEN), &OWNER, &forged), None);
        assert_eq!(
            verify(
                &signing_key(TOKEN),
                &OWNER,
                &stored(&OWNER, MAX_CONTENT_OBJECT_BYTES + 1)
            ),
            None
        );
    }

    #[test]
    fn integrity_requires_exact_size_and_digest() {
        let body = br#"{"messages":"m","response":"r"}"#;
        let digest = format!("{:x}", Sha256::digest(body));
        assert!(intact(body, body.len() as u64, &digest));
        assert!(!intact(body, body.len() as u64 + 1, &digest));
        assert!(!intact(
            br#"{"messages":"x","response":"r"}"#,
            body.len() as u64,
            &digest
        ));
    }

    #[rstest]
    #[case::full(0, "Input: \u{754c}\u{754c}\nOutput: o\nError: e", 0)]
    #[case::short_first_page(1, "Input: \u{754c}\u{754c}\nOutput: o\nError: e", 0)]
    #[case::offset_counts_characters(10, "\u{754c}\nOutput: o\nError: e", 0)]
    fn pages_count_unicode_characters_like_clickhouse(
        #[case] offset: u32,
        #[case] expected: &str,
        #[case] truncated: u8,
    ) {
        assert_eq!(
            page("\u{754c}\u{754c}", "o", "e", offset),
            (expected.into(), truncated)
        );
    }

    #[test]
    fn long_first_page_uses_bounded_excerpts() {
        let input = "\u{754c}".repeat(9000);
        let (content, truncated) = page(&input, "", "", 1);
        assert_eq!(truncated, 1);
        let head = format!(
            "Input: {}\n[... content omitted ...]\n",
            "\u{754c}".repeat(666)
        );
        assert!(content.starts_with(&head));
        assert_eq!(content.chars().count(), 24 + 2000 + 27);
        let (later, more) = page(&input, "", "", 9001);
        assert_eq!(later.chars().count(), 25);
        assert_eq!(more, 0);
    }

    #[test]
    fn references_signed_by_the_gateway_fixture_verify() {
        let key = signing_key("fixture-service-token-with-32-chars!");
        let owner = Owner {
            team_id: "team-\u{e9}",
            api_key: "hash-a",
            request_id: "request:1",
        };
        let stored = serde_json::json!({
            "v": 1,
            "state": "stored",
            "object_key": "logs/lens-content/2023-11-14/abc.json",
            "bytes": 45,
            "sha256": "0e469198a46d22437fab286514c9c8a42c17552d50e8613225e2c31bd5407968",
            "mac": "7bfcd946ff46db7efbd3b916275e80a21558196308a27108c1f8004f24e26d0f",
        })
        .to_string();
        assert!(matches!(
            verify(&key, &owner, &stored),
            Some(ContentReference::Stored { bytes: 45, .. })
        ));
        let unavailable = serde_json::json!({
            "v": 1,
            "state": "unavailable",
            "reason": "upload_failed",
            "mac": "64836be49c032f404b9a140b82c67fe8fdbb5dc2483583fe5aee11e3926e9a12",
        })
        .to_string();
        assert_eq!(
            verify(&key, &owner, &unavailable),
            Some(ContentReference::Unavailable("upload_failed".into()))
        );
        let other = Owner {
            team_id: "team-e",
            ..owner
        };
        assert_eq!(verify(&key, &other, &stored), None);
        assert_eq!(verify(&key, &other, &unavailable), None);
    }
}
