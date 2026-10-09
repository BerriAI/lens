use super::{Change, ClickHouseState, transport::encode};
use crate::Error;
use std::collections::BTreeMap;

impl ClickHouseState {
    pub async fn compact(&self, keys: &[&str]) -> Result<(), Error> {
        if keys.is_empty() {
            return Ok(());
        }
        if keys.len() > 100 {
            return Err(Error::InvalidState);
        }
        let changes = self
            .read_many(keys)
            .await?
            .into_iter()
            .map(|previous| Change {
                value: previous.value.clone(),
                previous,
            })
            .collect();
        let prepared = self.prepare(changes).await?;
        self.publish(&prepared).await?;
        let versions: BTreeMap<_, _> = prepared
            .blobs
            .iter()
            .map(|blob| (&blob.head.key, blob.head.revision))
            .collect();
        let digests: BTreeMap<_, _> = prepared
            .blobs
            .iter()
            .map(|blob| (&blob.head.key, &blob.head.digest))
            .collect();
        for table in [
            "lens_state_blobs",
            "lens_schedule",
            "lens_jobs",
            "lens_workers",
        ] {
            match self
                .command(
                    "EXISTS TABLE {table:Identifier}",
                    &[("table", table.into())],
                    String::new(),
                )
                .await?
                .trim()
            {
                "0" => continue,
                "1" => (),
                _ => return Err(Error::InvalidResponse),
            }
            self.command(
                "ALTER TABLE {table:Identifier} DELETE WHERE \
             key IN JSONExtract({keys:String}, 'Array(String)') AND \
             (revision < JSONExtract({versions:String}, 'Map(String, UInt64)')[key] OR \
             (revision = JSONExtract({versions:String}, 'Map(String, UInt64)')[key] AND \
             digest != JSONExtract({digests:String}, 'Map(String, String)')[key])) \
             SETTINGS mutations_sync=2",
                &[
                    ("table", table.into()),
                    ("keys", encode(&keys)?),
                    ("versions", encode(&versions)?),
                    ("digests", encode(&digests)?),
                ],
                String::new(),
            )
            .await?;
        }
        Ok(())
    }
}
