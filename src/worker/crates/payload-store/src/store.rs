use aws_sdk_s3::{
    Client,
    operation::get_object::GetObjectOutput,
    primitives::ByteStream,
    types::{Delete, ObjectIdentifier},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Days, Utc};
use sha2::{Digest, Sha256};

use crate::{
    Error, ObjectKey,
    codec::{BYTES_METADATA, ENCODING_METADATA, Encoding, MAX_BODY_BYTES, decode, encode},
    key::{Generated, NAMESPACE, day_path},
};

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreConfig {
    pub bucket: String,
    pub prefix: String,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub path_style: bool,
}

impl StoreConfig {
    pub fn from_lookup(read: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, Error> {
        let Some(bucket) = read("LENS_PAYLOAD_S3_BUCKET").filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let prefix = read("LENS_PAYLOAD_S3_PREFIX").unwrap_or_default();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            return Err(Error::Configuration(
                "LENS_PAYLOAD_S3_PREFIX must be empty or end with /",
            ));
        }
        let path_style = match read("LENS_PAYLOAD_S3_PATH_STYLE").as_deref() {
            None | Some("" | "false" | "0") => false,
            Some("true" | "1") => true,
            Some(_) => {
                return Err(Error::Configuration(
                    "LENS_PAYLOAD_S3_PATH_STYLE must be true or false",
                ));
            }
        };
        Ok(Some(Self {
            bucket,
            prefix,
            endpoint: read("LENS_PAYLOAD_S3_ENDPOINT").filter(|value| !value.is_empty()),
            region: read("LENS_PAYLOAD_S3_REGION").filter(|value| !value.is_empty()),
            path_style,
        }))
    }
}

fn request_error<E: std::error::Error + Send + Sync + 'static>(error: E) -> Error {
    Error::Request(Box::new(error))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(request_error)?
}

fn metadata<'a>(object: &'a GetObjectOutput, name: &str) -> Option<&'a str> {
    object
        .metadata()
        .and_then(|metadata| metadata.get(name))
        .map(String::as_str)
}

async fn bounded(mut body: ByteStream, limit: u64) -> Result<Vec<u8>, Error> {
    let mut stored = Vec::new();
    while let Some(chunk) = body.try_next().await.map_err(request_error)? {
        if (stored.len() + chunk.len()) as u64 > limit {
            return Err(Error::Integrity);
        }
        stored.extend_from_slice(&chunk);
    }
    Ok(stored)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sweep {
    pub deleted: u64,
    pub failed: u64,
}

pub struct Store {
    client: Client,
    bucket: String,
    prefix: String,
}

impl Store {
    pub fn new(client: Client, bucket: String, prefix: String) -> Self {
        Self {
            client,
            bucket,
            prefix,
        }
    }

    pub async fn connect(config: StoreConfig) -> Self {
        let shared = aws_config::load_from_env().await;
        let builder =
            aws_sdk_s3::config::Builder::from(&shared).force_path_style(config.path_style);
        let builder = match config.endpoint {
            Some(endpoint) => builder.endpoint_url(endpoint),
            None => builder,
        };
        let builder = match config.region {
            Some(region) => builder.region(aws_sdk_s3::config::Region::new(region)),
            None => builder,
        };
        Self::new(
            Client::from_conf(builder.build()),
            config.bucket,
            config.prefix,
        )
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Stores `body` gzip-compressed when that is smaller, recording the encoding and the logical
    /// length in object metadata. The checksum S3 verifies covers the stored bytes.
    pub async fn put(&self, key: &ObjectKey, body: Vec<u8>, sha256: &str) -> Result<(), Error> {
        if body.len() as u64 > MAX_BODY_BYTES {
            return Err(Error::Integrity);
        }
        let expected = sha256.to_owned();
        let (stored, logical, checksum) = blocking(move || {
            if sha256_hex(&body) != expected {
                return Err(Error::Integrity);
            }
            let logical = body.len();
            let stored = encode(body);
            let checksum = STANDARD.encode(Sha256::digest(&stored.bytes));
            Ok((stored, logical, checksum))
        })
        .await?;
        let length = i64::try_from(stored.bytes.len()).map_err(|_| Error::Integrity)?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .content_length(length)
            .checksum_sha256(checksum)
            .content_type(stored.encoding.content_type())
            .metadata(ENCODING_METADATA, stored.encoding.name())
            .metadata(BYTES_METADATA, logical.to_string())
            .body(ByteStream::from(stored.bytes))
            .send()
            .await
            .map_err(request_error)?;
        Ok(())
    }

    /// Reads exactly `bytes` logical bytes matching `sha256`. Objects without encoding metadata
    /// were stored as is. The stored body is never read past `bytes`.
    pub async fn get(&self, key: &ObjectKey, bytes: u64, sha256: &str) -> Result<String, Error> {
        if bytes > MAX_BODY_BYTES {
            return Err(Error::Integrity);
        }
        let object = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|error| match error.into_service_error() {
                service if service.is_no_such_key() => Error::Missing,
                service => request_error(service),
            })?;
        let encoding =
            Encoding::parse(metadata(&object, ENCODING_METADATA)).ok_or(Error::Integrity)?;
        if metadata(&object, BYTES_METADATA).is_some_and(|recorded| recorded != bytes.to_string())
            || object
                .content_length()
                .is_some_and(|length| !u64::try_from(length).is_ok_and(|length| length <= bytes))
        {
            return Err(Error::Integrity);
        }
        let stored = bounded(object.body, bytes).await?;
        let expected = sha256.to_owned();
        let body = blocking(move || {
            let body = decode(stored, encoding, bytes)?;
            (sha256_hex(&body) == expected)
                .then_some(body)
                .ok_or(Error::Integrity)
        })
        .await?;
        String::from_utf8(body).map_err(|_| Error::Integrity)
    }

    /// Deletes every object Lens generated for a day before `today - retention_days`. Listing is
    /// in key order, so it walks expired days oldest first and stops at the first retained day:
    /// each pass reads what has expired since the last one, however long Lens was down.
    pub async fn sweep(&self, today: DateTime<Utc>, retention_days: u64) -> Result<Sweep, Error> {
        let Some(cutoff) = today
            .date_naive()
            .checked_sub_days(Days::new(retention_days))
        else {
            return Ok(Sweep::default());
        };
        let namespace = format!("{}{NAMESPACE}", self.prefix);
        let retained = format!("{namespace}{}/", day_path(cutoff));
        let mut sweep = Sweep::default();
        let mut token = None;
        loop {
            let page = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&namespace)
                .set_continuation_token(token)
                .send()
                .await
                .map_err(request_error)?;
            let keys: Vec<&str> = page
                .contents()
                .iter()
                .filter_map(|object| object.key())
                .collect();
            let reached_retained = keys.iter().any(|key| *key >= retained.as_str());
            let expired: Vec<&str> = keys
                .into_iter()
                .filter(|key| {
                    Generated::parse(&self.prefix, key).is_some_and(|key| key.day < cutoff)
                })
                .collect();
            let failed = self.delete(&expired).await?;
            sweep.deleted += (expired.len() as u64).saturating_sub(failed);
            sweep.failed += failed;
            token = page.next_continuation_token().map(str::to_owned);
            if reached_retained || token.is_none() {
                return Ok(sweep);
            }
        }
    }

    async fn delete(&self, keys: &[&str]) -> Result<u64, Error> {
        if keys.is_empty() {
            return Ok(0);
        }
        let identifiers = keys
            .iter()
            .map(|key| {
                ObjectIdentifier::builder()
                    .key(*key)
                    .build()
                    .map_err(request_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let output = self
            .client
            .delete_objects()
            .bucket(&self.bucket)
            .delete(
                Delete::builder()
                    .set_objects(Some(identifiers))
                    .quiet(true)
                    .build()
                    .map_err(request_error)?,
            )
            .send()
            .await
            .map_err(request_error)?;
        Ok(output.errors().len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use std::collections::BTreeMap;

    use super::*;

    fn lookup(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let values: BTreeMap<String, String> = values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| values.get(name).cloned()
    }

    #[rstest]
    fn store_is_disabled_without_a_bucket() {
        assert_eq!(StoreConfig::from_lookup(lookup(&[])).unwrap(), None);
    }

    #[rstest]
    fn store_reads_operator_settings() {
        let config = StoreConfig::from_lookup(lookup(&[
            ("LENS_PAYLOAD_S3_BUCKET", "lens"),
            ("LENS_PAYLOAD_S3_PREFIX", "payloads/"),
            ("LENS_PAYLOAD_S3_ENDPOINT", "http://minio:9000"),
            ("LENS_PAYLOAD_S3_PATH_STYLE", "true"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            config,
            StoreConfig {
                bucket: "lens".into(),
                prefix: "payloads/".into(),
                endpoint: Some("http://minio:9000".into()),
                region: None,
                path_style: true,
            }
        );
    }

    #[rstest]
    #[case::prefix(("LENS_PAYLOAD_S3_PREFIX", "payloads"))]
    #[case::path_style(("LENS_PAYLOAD_S3_PATH_STYLE", "yes"))]
    fn invalid_settings_are_rejected(#[case] setting: (&str, &str)) {
        assert!(matches!(
            StoreConfig::from_lookup(lookup(&[("LENS_PAYLOAD_S3_BUCKET", "lens"), setting])),
            Err(Error::Configuration(_))
        ));
    }
}
