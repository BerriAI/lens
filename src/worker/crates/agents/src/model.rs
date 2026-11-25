use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    String,
    Number,
    Boolean,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DefinitionInput {
    pub key: String,
    pub description: String,
    pub kind: ValueKind,
    #[serde(default)]
    pub allowed_values: Vec<Value>,
}

impl DefinitionInput {
    pub fn validate(&self) -> Result<(), Error> {
        if self.key.is_empty()
            || self.key.len() > 80
            || !self
                .key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::Invalid(
                "Use a lowercase annotation key with letters, numbers and underscores (up to 80 characters)",
            ));
        }
        bounded_text(&self.description, 2000)?;
        if self.allowed_values.len() > 100
            || self
                .allowed_values
                .iter()
                .any(|value| !self.accepts_type(value))
        {
            return Err(Error::Invalid(
                "Allowed values must match the definition's type (up to 100 values)",
            ));
        }
        Ok(())
    }

    fn accepts_type(&self, value: &Value) -> bool {
        match self.kind {
            ValueKind::String => value.as_str().is_some_and(|s| s.len() <= 4096),
            ValueKind::Number => value.is_number(),
            ValueKind::Boolean => value.is_boolean(),
        }
    }

    pub fn accepts(&self, value: &Value) -> bool {
        self.accepts_type(value)
            && (self.allowed_values.is_empty() || self.allowed_values.contains(value))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Definition {
    pub id: String,
    #[serde(flatten)]
    pub definition: DefinitionInput,
    pub aliases: Vec<String>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TraceIdentity {
    pub trace_id: String,
    pub trace_ref: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnnotationInput {
    pub trace: TraceIdentity,
    pub definition_id: String,
    pub value: Value,
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Annotation {
    pub definition_key: String,
    pub producer_name: String,
    #[serde(flatten)]
    pub annotation: AnnotationInput,
    pub producer_id: String,
    pub execution_id: String,
    pub updated_at: i64,
}

#[derive(Default, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnnotationFilter {
    pub definition_id: Option<String>,
    pub producer_id: Option<String>,
    pub value: Option<Value>,
    pub trace_id: Option<String>,
    pub trace_ref: Option<String>,
    pub after: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AnnotationPage {
    pub annotations: Vec<Annotation>,
    pub next_cursor: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnnotationActivity {
    pub producer_id: String,
    pub producer_name: String,
    pub definition_id: String,
    pub definition_key: String,
    pub value: Value,
    pub count: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportInput {
    pub id: String,
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(default)]
    pub traces: Vec<TraceIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Report {
    pub preview: bool,
    pub artifacts: Vec<crate::artifacts::ArtifactInfo>,
    #[serde(flatten)]
    pub report: ReportInput,
    pub producer_id: String,
    pub execution_id: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub workspace: String,
    pub producer_id: String,
    pub execution_id: String,
}

pub(crate) fn bounded_text(text: &str, max: usize) -> Result<(), Error> {
    if text.trim().is_empty() || text.len() > max {
        return Err(Error::Invalid("Text is empty or too long"));
    }
    Ok(())
}
