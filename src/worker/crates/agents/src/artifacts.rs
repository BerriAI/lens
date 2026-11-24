use base64::{Engine, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::{
    Error,
    model::{Actor, bounded_text},
    store::Store,
};

pub const MAX_ARTIFACT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishArtifact {
    pub id: String,
    pub path: String,
    pub filename: String,
    pub media_type: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ArtifactInfo {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub size: usize,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
pub struct Artifact {
    #[serde(flatten)]
    pub info: ArtifactInfo,
    pub data_base64: String,
}

impl Store {
    pub async fn save_artifact(
        &self,
        actor: &Actor,
        input: &PublishArtifact,
        content: &[u8],
    ) -> Result<ArtifactInfo, Error> {
        bounded_text(&input.id, 128)?;
        bounded_text(&input.filename, 200)?;
        if input.filename.contains(['/', '\\', '\r', '\n', '\0'])
            || content.len() > MAX_ARTIFACT_BYTES
            || content.is_empty()
        {
            return Err(Error::Invalid(
                "Use a filename and an artifact between 1 byte and 10 MiB",
            ));
        }
        let valid = match input.media_type.as_str() {
            "image/png" => content.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => content.starts_with(b"\xff\xd8\xff"),
            "image/webp" => content.starts_with(b"RIFF") && content.get(8..12) == Some(b"WEBP"),
            "text/csv" | "text/plain" | "text/markdown" => std::str::from_utf8(content).is_ok(),
            "application/json" => serde_json::from_slice::<serde_json::Value>(content).is_ok(),
            _ => false,
        };
        if !valid {
            return Err(Error::Invalid(
                "Publish a PNG, JPEG, WebP, UTF-8 text, CSV, Markdown or JSON artifact with its matching media type",
            ));
        }
        let digest = format!("{:x}", Sha256::digest(content));
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        let existing = sqlx::query("SELECT producer_id,digest,filename,media_type FROM artifacts WHERE workspace=? AND id=?")
            .bind(&actor.workspace).bind(&input.id).fetch_optional(&mut *tx).await?;
        if let Some(existing) = existing {
            if existing.try_get::<&str, _>("producer_id")? != actor.producer_id
                || existing.try_get::<&str, _>("digest")? != digest
                || existing.try_get::<&str, _>("filename")? != input.filename
                || existing.try_get::<&str, _>("media_type")? != input.media_type
            {
                return Err(Error::Conflict(
                    "This artifact ID already identifies a different file. Publish the new version with a new ID",
                ));
            }
        } else {
            sqlx::query("INSERT INTO artifacts VALUES (?,?,?,?,?,?,?,?)")
                .bind(&actor.workspace)
                .bind(&input.id)
                .bind(&actor.producer_id)
                .bind(&input.filename)
                .bind(&input.media_type)
                .bind(content)
                .bind(&digest)
                .bind(chrono::Utc::now().timestamp_millis())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ArtifactInfo {
            id: input.id.clone(),
            filename: input.filename.clone(),
            media_type: input.media_type.clone(),
            size: content.len(),
            sha256: digest,
        })
    }

    pub async fn artifact(&self, workspace: &str, id: &str) -> Result<Artifact, Error> {
        let row = sqlx::query("SELECT * FROM artifacts WHERE workspace=? AND id=?")
            .bind(workspace)
            .bind(id)
            .fetch_optional(&self.0)
            .await?
            .ok_or(Error::NotFound)?;
        let bytes: Vec<u8> = row.try_get("content")?;
        Ok(Artifact {
            info: ArtifactInfo {
                id: id.into(),
                filename: row.try_get("filename")?,
                media_type: row.try_get("media_type")?,
                size: bytes.len(),
                sha256: row.try_get("digest")?,
            },
            data_base64: STANDARD.encode(bytes),
        })
    }

    pub async fn report_artifacts(
        &self,
        workspace: &str,
        id: &str,
    ) -> Result<Vec<ArtifactInfo>, Error> {
        sqlx::query("SELECT a.id,a.filename,a.media_type,length(a.content) AS size,a.digest FROM artifacts a JOIN report_artifacts r ON r.workspace=a.workspace AND r.artifact_id=a.id WHERE r.workspace=? AND r.report_id=? ORDER BY r.position")
            .bind(workspace).bind(id).fetch_all(&self.0).await?.iter().map(|row|Ok(ArtifactInfo {
                id:row.try_get("id")?,filename:row.try_get("filename")?,media_type:row.try_get("media_type")?,size:row.try_get::<u32,_>("size")? as usize,sha256:row.try_get("digest")?,
            })).collect()
    }
}
