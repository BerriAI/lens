#![forbid(unsafe_code)]

pub mod clustering;
mod error;

use std::path::PathBuf;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use error::Error;

pub const MAX_CLUSTERS: usize = 64;
pub const MAX_TASK_BYTES: usize = 1024;
pub const DIMENSIONS: usize = 384;
pub const MODEL_VERSION: u32 = 1;
pub const UNCERTAIN_LABEL: &str = "Other / uncertain";
const MATCH_THRESHOLD: f32 = 0.65;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCluster {
    pub id: String,
    pub label: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub cluster_id: String,
    pub label: String,
    pub new_cluster: Option<TaskCluster>,
}

pub struct Classifier {
    cache_dir: PathBuf,
    model: Option<TextEmbedding>,
}

impl Classifier {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            model: None,
        }
    }

    pub fn classify(&mut self, text: &str, clusters: &[TaskCluster]) -> Result<Assignment, Error> {
        validate_clusters(clusters)?;
        let task = task_description(text);
        if !supported_task(&task) {
            return Ok(uncertain());
        }
        if self.model.is_none() {
            self.model = Some(
                TextEmbedding::try_new(
                    TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
                        .with_cache_dir(self.cache_dir.clone())
                        .with_show_download_progress(false)
                        .with_max_length(128)
                        .with_intra_threads(1)
                        .with_session_config("session.intra_op.allow_spinning", "0")
                        .with_session_config("session.inter_op.allow_spinning", "0"),
                )
                .map_err(Error::Model)?,
            );
        }
        let embedding = self
            .model
            .as_mut()
            .ok_or(Error::InvalidEmbedding)?
            .embed([&task], Some(1))
            .map_err(Error::Model)?
            .pop()
            .ok_or(Error::InvalidEmbedding)?;
        assign(&task, embedding, clusters)
    }
}

pub fn task_description(text: &str) -> String {
    let mut end = text.len().min(MAX_TASK_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .scan(0, |bytes, character| {
            *bytes += character.len_utf8();
            (*bytes <= MAX_TASK_BYTES).then_some(character)
        })
        .collect()
}

fn supported_task(task: &str) -> bool {
    !task.is_empty()
        && !task
            .chars()
            .any(|character| character.is_alphabetic() && character > '\u{024f}')
}

pub fn assign(
    text: &str,
    embedding: Vec<f32>,
    clusters: &[TaskCluster],
) -> Result<Assignment, Error> {
    validate_clusters(clusters)?;
    if !valid_embedding(&embedding) {
        return Err(Error::InvalidEmbedding);
    }
    let task = task_description(text);
    if !supported_task(&task) {
        return Ok(uncertain());
    }
    let nearest = clusters
        .iter()
        .map(|cluster| (cluster, cosine(&embedding, &cluster.embedding)))
        .max_by(|(left_cluster, left), (right_cluster, right)| {
            left.total_cmp(right)
                .then_with(|| right_cluster.id.cmp(&left_cluster.id))
        });
    if let Some((cluster, similarity)) = nearest
        && similarity >= MATCH_THRESHOLD
    {
        return Ok(Assignment {
            cluster_id: cluster.id.clone(),
            label: cluster.label.clone(),
            new_cluster: None,
        });
    }
    if clusters.len() == MAX_CLUSTERS {
        return Ok(uncertain());
    }
    let cluster = TaskCluster {
        id: format!("{:x}", Sha256::digest(task.as_bytes())),
        label: keyword_label(&task),
        embedding,
    };
    Ok(Assignment {
        cluster_id: cluster.id.clone(),
        label: cluster.label.clone(),
        new_cluster: Some(cluster),
    })
}

fn uncertain() -> Assignment {
    Assignment {
        cluster_id: String::new(),
        label: UNCERTAIN_LABEL.into(),
        new_cluster: None,
    }
}

fn validate_clusters(clusters: &[TaskCluster]) -> Result<(), Error> {
    if clusters.len() > MAX_CLUSTERS
        || clusters.iter().any(|cluster| {
            cluster.id.len() != 64
                || !cluster.id.bytes().all(|byte| byte.is_ascii_hexdigit())
                || cluster.label.len() > 96
                || cluster.label.is_empty()
                || !valid_embedding(&cluster.embedding)
        })
    {
        return Err(Error::InvalidClusters);
    }
    Ok(())
}

fn valid_embedding(embedding: &[f32]) -> bool {
    embedding.len() == DIMENSIONS
        && embedding.iter().all(|value| value.is_finite())
        && embedding.iter().map(|value| value * value).sum::<f32>() > 0.0
        && embedding.iter().all(|value| value.abs() <= 1.0)
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot: f32 = left.iter().zip(right).map(|(a, b)| a * b).sum();
    let left_norm: f32 = left.iter().map(|value| value * value).sum();
    let right_norm: f32 = right.iter().map(|value| value * value).sum();
    dot / (left_norm * right_norm).sqrt()
}

fn keyword_label(task: &str) -> String {
    let words = task
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() > 1)
        .filter(|word| {
            !matches!(
                *word,
                "please"
                    | "can"
                    | "could"
                    | "you"
                    | "the"
                    | "this"
                    | "that"
                    | "for"
                    | "and"
                    | "with"
                    | "from"
                    | "help"
                    | "need"
                    | "want"
                    | "would"
                    | "make"
                    | "create"
                    | "generate"
                    | "give"
                    | "some"
                    | "into"
            )
        })
        .take(5)
        .collect::<Vec<_>>()
        .join(" ");
    if words.is_empty() {
        return "General tasks".into();
    }
    let mut label = String::new();
    for character in words.chars() {
        if label.len() + character.len_utf8() > 96 {
            break;
        }
        label.push(character);
    }
    label
}
