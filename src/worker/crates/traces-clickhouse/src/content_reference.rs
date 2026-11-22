//! Signed references LiteLLM puts on a proxy-generated trace span whose content it stored in object storage.
//! Each reference is bound to the span's team, key, trace and span, so it cannot be replayed onto another row.

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const REFERENCE_ATTRIBUTE: &str = "litellm.lens.content";
pub const MAX_CONTENT_OBJECT_BYTES: u64 = 16 * 1024 * 1024;
const VERSION: u8 = 2;

pub struct Owner<'a> {
    pub team_id: &'a str,
    pub api_key: &'a str,
    pub trace_id: &'a str,
    pub span_id: &'a str,
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
        "2",
        owner.team_id,
        owner.api_key,
        owner.trace_id,
        owner.span_id,
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

/// The reference's content state when it is signed for exactly this span's owner, else None.
pub fn verify(key: &[u8], owner: &Owner<'_>, raw: &str) -> Option<ContentReference> {
    match serde_json::from_str::<Signed>(raw).ok()? {
        Signed::Stored {
            v: VERSION,
            object_key,
            bytes,
            sha256,
            mac,
        } if bytes <= MAX_CONTENT_OBJECT_BYTES
            && authentic(
                key,
                owner,
                ["stored", "", &object_key, &bytes.to_string(), &sha256],
                &mac,
            ) =>
        {
            Some(ContentReference::Stored {
                object_key,
                bytes,
                sha256,
            })
        }
        Signed::Unavailable {
            v: VERSION,
            reason,
            mac,
        } if authentic(key, owner, ["unavailable", &reason, "", "", ""], &mac) => {
            Some(ContentReference::Unavailable(reason))
        }
        _ => None,
    }
}

pub fn intact(body: &[u8], bytes: u64, sha256: &str) -> bool {
    u64::try_from(body.len()).is_ok_and(|length| length == bytes)
        && format!("{:x}", Sha256::digest(body)) == sha256
}

const PAGE_CHARS: usize = 8000;

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

/// One page of a span's investigation content, exactly as `lens_content.sql` pages inline content.
pub fn page(
    input: &str,
    output: &str,
    status_code: &str,
    status_message: &str,
    offset: u32,
) -> (String, u8) {
    let full = format!("Input: {input}\nOutput: {output}\nStatus: {status_code} {status_message}");
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
                "Input: {}\nOutput: {}\nStatus: {status_code} {}",
                excerpt(input, 2000),
                excerpt(output, 5000),
                excerpt(status_message, 500)
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

    const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
    const SPAN: &str = "b7ad6b7169203331";

    fn owner(team_id: &str) -> Owner<'_> {
        Owner {
            team_id,
            api_key: "hash-a",
            trace_id: TRACE,
            span_id: SPAN,
        }
    }

    fn fixture(state: &str) -> String {
        let stored = serde_json::json!({
            "v": 2, "state": "stored", "object_key": "logs/lens-content/2023-11-14/abc.json", "bytes": 45,
            "sha256": "0e469198a46d22437fab286514c9c8a42c17552d50e8613225e2c31bd5407968",
            "mac": "c2172cc69bbddfc0003ccea9602142710e9f4e3a9a639f62d18715f667ce0a0f",
        });
        let unavailable = serde_json::json!({
            "v": 2, "state": "unavailable", "reason": "upload_failed",
            "mac": "b691bdbb706af2a54e56f68077a6a63c909d5e6248aa37c1764feff4d88f30b9",
        });
        if state == "stored" {
            stored
        } else {
            unavailable
        }
        .to_string()
    }

    #[test]
    fn references_signed_by_litellm_verify_for_their_own_span_only() {
        let key = signing_key("fixture-service-token-with-32-chars!");
        assert_eq!(
            verify(&key, &owner("team-\u{e9}"), &fixture("stored")),
            Some(ContentReference::Stored {
                object_key: "logs/lens-content/2023-11-14/abc.json".into(),
                bytes: 45,
                sha256: "0e469198a46d22437fab286514c9c8a42c17552d50e8613225e2c31bd5407968".into(),
            })
        );
        assert_eq!(
            verify(&key, &owner("team-\u{e9}"), &fixture("unavailable")),
            Some(ContentReference::Unavailable("upload_failed".into()))
        );
        let other_span = Owner {
            span_id: "0000000000000000",
            ..owner("team-\u{e9}")
        };
        for (owner, raw) in [
            (owner("team-b"), fixture("stored")),
            (other_span, fixture("stored")),
            (
                owner("team-\u{e9}"),
                fixture("stored").replace("\"bytes\":45", "\"bytes\":46"),
            ),
            (
                owner("team-\u{e9}"),
                fixture("stored").replace("\"v\":2", "\"v\":1"),
            ),
        ] {
            assert_eq!(verify(&key, &owner, &raw), None);
        }
        let other_key = signing_key("another-service-token-with-32-chars");
        assert_eq!(
            verify(&other_key, &owner("team-\u{e9}"), &fixture("stored")),
            None
        );
    }

    #[test]
    fn content_must_match_the_signed_size_and_digest() {
        let body = br#"{"gen_ai.input.messages":"[]"}"#;
        let digest = format!("{:x}", Sha256::digest(body));
        assert!(intact(body, body.len() as u64, &digest));
        assert!(!intact(body, body.len() as u64 + 1, &digest));
        assert!(!intact(b"{}", body.len() as u64, &digest));
    }

    #[test]
    fn hydrated_content_pages_like_inline_content() {
        assert_eq!(
            page("ask", "answer", "Unset", "", 0),
            ("Input: ask\nOutput: answer\nStatus: Unset ".into(), 0)
        );
        let long = "界".repeat(20000);
        let (first, truncated) = page(&long, "done", "Unset", "", 1);
        assert_eq!(truncated, 1);
        assert!(first.contains("[... content omitted ...]"));
        assert!(first.ends_with("Output: done\nStatus: Unset "));
        let (second, _) = page(&long, "done", "Unset", "", 8001);
        assert_eq!(second.chars().count(), 8000);
        assert!(second.chars().all(|character| character == '界'));
    }
}
