use std::io::{Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::Error;

/// Largest logical body Lens stores or reads back, matching its 32 MiB request limit.
pub const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const ENCODING_METADATA: &str = "lens-encoding";
pub(crate) const BYTES_METADATA: &str = "lens-bytes";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Encoding {
    Identity,
    Gzip,
}

impl Encoding {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Gzip => "gzip",
        }
    }

    pub(crate) fn content_type(self) -> &'static str {
        match self {
            Self::Identity => "application/json",
            Self::Gzip => "application/gzip",
        }
    }

    /// Objects written before compression carry no encoding and are stored as is.
    pub(crate) fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("identity") => Some(Self::Identity),
            Some("gzip") => Some(Self::Gzip),
            Some(_) => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Stored {
    pub(crate) bytes: Vec<u8>,
    pub(crate) encoding: Encoding,
}

pub(crate) fn encode(body: Vec<u8>) -> Stored {
    match gzip(&body) {
        Some(compressed) if compressed.len() < body.len() => Stored {
            bytes: compressed,
            encoding: Encoding::Gzip,
        },
        _ => Stored {
            bytes: body,
            encoding: Encoding::Identity,
        },
    }
}

fn gzip(body: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(body.len() / 2), Compression::default());
    encoder.write_all(body).ok()?;
    encoder.finish().ok()
}

/// Recovers exactly `logical` bytes. A compressed object is only ever stored when smaller than
/// its body, and decompression stops one byte past `logical`, so a corrupt or oversized object
/// fails without unbounded allocation.
pub(crate) fn decode(stored: Vec<u8>, encoding: Encoding, logical: u64) -> Result<Vec<u8>, Error> {
    if logical > MAX_BODY_BYTES {
        return Err(Error::Integrity);
    }
    let expected = usize::try_from(logical).map_err(|_| Error::Integrity)?;
    let body = match encoding {
        Encoding::Identity => stored,
        Encoding::Gzip if stored.len() >= expected => return Err(Error::Integrity),
        Encoding::Gzip => {
            let mut body = Vec::with_capacity(expected);
            GzDecoder::new(stored.as_slice())
                .take(logical + 1)
                .read_to_end(&mut body)
                .map_err(|_| Error::Integrity)?;
            body
        }
    };
    (body.len() == expected)
        .then_some(body)
        .ok_or(Error::Integrity)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn compressible() -> Vec<u8> {
        "雪 snow ".repeat(4096).into_bytes()
    }

    #[rstest]
    fn repetitive_bodies_are_stored_compressed_and_read_back_exactly() {
        let body = compressible();
        let stored = encode(body.clone());

        assert_eq!(stored.encoding, Encoding::Gzip);
        assert!(stored.bytes.len() < body.len());
        assert_eq!(
            decode(stored.bytes, stored.encoding, body.len() as u64).unwrap(),
            body
        );
    }

    #[rstest]
    #[case::empty(b"".to_vec())]
    #[case::short_body(b"[\"\xe9\x9b\xaa\"]".to_vec())]
    fn bodies_compression_cannot_shrink_are_stored_as_is(#[case] body: Vec<u8>) {
        assert_eq!(
            encode(body.clone()),
            Stored {
                bytes: body,
                encoding: Encoding::Identity,
            }
        );
    }

    #[rstest]
    fn compression_is_deterministic_so_retried_uploads_store_the_same_object() {
        assert_eq!(encode(compressible()), encode(compressible()));
    }

    #[rstest]
    #[case::truncated(|bytes: Vec<u8>| bytes[..bytes.len() / 2].to_vec(), 0)]
    #[case::corrupt(|mut bytes: Vec<u8>| { let middle = bytes.len() / 2; bytes[middle] ^= 0xff; bytes }, 0)]
    #[case::decompresses_past_its_length(|bytes| bytes, -1)]
    #[case::decompresses_short_of_its_length(|bytes| bytes, 1)]
    #[case::not_gzip(|bytes: Vec<u8>| vec![b'x'; bytes.len()], 0)]
    fn damaged_compressed_objects_are_rejected(
        #[case] damage: fn(Vec<u8>) -> Vec<u8>,
        #[case] length_offset: i64,
    ) {
        let body = compressible();
        let logical = body.len() as u64;
        let claimed = logical.checked_add_signed(length_offset).unwrap();

        assert!(matches!(
            decode(damage(encode(body).bytes), Encoding::Gzip, claimed),
            Err(Error::Integrity)
        ));
    }

    #[rstest]
    #[case::compressed_no_smaller_than_its_body(Encoding::Gzip, 4)]
    #[case::raw_of_another_length(Encoding::Identity, 5)]
    #[case::above_the_body_limit(Encoding::Identity, MAX_BODY_BYTES + 1)]
    fn objects_that_cannot_hold_the_recorded_body_are_rejected(
        #[case] encoding: Encoding,
        #[case] logical: u64,
    ) {
        assert!(matches!(
            decode(b"abcd".to_vec(), encoding, logical),
            Err(Error::Integrity)
        ));
    }

    #[rstest]
    #[case::historic(None, Some(Encoding::Identity))]
    #[case::identity(Some("identity"), Some(Encoding::Identity))]
    #[case::gzip(Some("gzip"), Some(Encoding::Gzip))]
    #[case::unknown(Some("br"), None)]
    fn encoding_metadata_is_explicit(
        #[case] value: Option<&str>,
        #[case] expected: Option<Encoding>,
    ) {
        assert_eq!(Encoding::parse(value), expected);
    }
}
