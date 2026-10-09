use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{DisplayFromStr, PickFirst, serde_as};
use sha2::{Digest, Sha256};

use crate::Error;

#[serde_as]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct Head {
    pub key: String,
    #[serde_as(as = "PickFirst<(_, DisplayFromStr)>")]
    pub revision: u64,
    pub digest: String,
}

impl Head {
    pub fn empty(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            revision: 0,
            digest: String::new(),
        }
    }

    pub(super) fn valid(&self) -> bool {
        if self.revision == 0 {
            return self.digest.is_empty();
        }
        self.digest.len() == 64
            && self
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    pub head: Head,
    pub value: Value,
}

impl Snapshot {
    pub fn empty(key: impl Into<String>) -> Self {
        Self {
            head: Head::empty(key),
            value: Value::Null,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Change {
    pub previous: Snapshot,
    pub value: Value,
}

#[derive(Debug)]
pub struct PreparedCommit {
    pub(super) changes: Vec<Change>,
    pub(super) blobs: Vec<Blob>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Blob {
    #[serde(flatten)]
    pub head: Head,
    pub data: String,
}

impl Blob {
    pub fn next(change: &Change) -> Result<Self, Error> {
        let data = super::transport::encode(&change.value)?;
        let revision = change
            .previous
            .head
            .revision
            .checked_add(1)
            .ok_or(Error::InvalidState)?;
        Ok(Self {
            head: Head {
                key: change.previous.head.key.clone(),
                revision,
                digest: digest(&data),
            },
            data,
        })
    }

    pub fn snapshot(self) -> Result<Snapshot, Error> {
        if self.head.digest != digest(&self.data) {
            return Err(Error::InvalidResponse);
        }
        let value = serde_json::from_str(&self.data).map_err(|_| Error::InvalidResponse)?;
        Ok(Snapshot {
            head: self.head,
            value,
        })
    }
}

fn digest(data: &str) -> String {
    format!("{:x}", Sha256::digest(data.as_bytes()))
}
