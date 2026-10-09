use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::{InvalidAgentIo, eval::TraceAttribute};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(extend("pattern" = r"^(?:/(?:[^~/]|~[01])*)*$"))]
pub struct JsonPointer(String);

impl TryFrom<String> for JsonPointer {
    type Error = InvalidAgentIo;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if (!value.is_empty() && !value.starts_with('/'))
            || value
                .split('~')
                .skip(1)
                .any(|part| !part.starts_with(['0', '1']))
        {
            return Err(InvalidAgentIo::JsonPointer);
        }
        Ok(Self(value))
    }
}

impl From<JsonPointer> for String {
    fn from(value: JsonPointer) -> Self {
        value.0
    }
}

impl JsonPointer {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn resolve<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        value.pointer(self.as_str())
    }

    pub fn resolve_mut<'a>(&self, value: &'a mut Value) -> Option<&'a mut Value> {
        value.pointer_mut(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentIo {
    #[schemars(extend("const" = 1))]
    pub version: u32,
    #[schemars(regex(pattern = r"^[a-zA-Z][a-zA-Z0-9_-]{0,63}$"))]
    pub connection: String,
    pub submit: AgentRequest,
    #[schemars(length(min = 1))]
    pub input: Vec<InputBinding>,
    pub completion: Completion,
    pub output: OutputMapping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceMapping>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SubmitMethod {
    #[serde(rename = "POST")]
    Post,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub method: SubmitMethod,
    pub path: String,
    pub json: Map<String, Value>,
    #[schemars(range(min = 200, max = 299))]
    pub accepted_status: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum InputSource {
    #[serde(rename = "case.input")]
    CaseInput,
    #[serde(rename = "case.followups")]
    CaseFollowups,
    #[serde(rename = "trial.request_id")]
    TrialRequestId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputBinding {
    pub source: InputSource,
    pub target: JsonPointer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Completion {
    #[schemars(title = "ImmediateCompletion")]
    Immediate {},
    #[schemars(title = "PollCompletion")]
    Poll(Box<PollCompletion>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PollCompletion {
    pub id_pointer: JsonPointer,
    pub path_template: String,
    pub status_pointer: JsonPointer,
    #[schemars(length(min = 1))]
    pub success: Vec<String>,
    #[schemars(length(min = 1))]
    pub failure: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_pointer: Option<JsonPointer>,
    #[schemars(range(min = 250, max = 60000))]
    pub interval_ms: u64,
    #[schemars(range(min = 1000, max = 3600000))]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_item: Option<RequiredItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredItem {
    pub array_pointer: JsonPointer,
    #[schemars(length(min = 1))]
    pub matches: BTreeMap<String, JsonScalar>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum JsonScalar {
    String(String),
    Number(Number),
    Bool(bool),
    Null,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputMapping {
    pub pointer: JsonPointer,
    pub require_nonempty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceSource {
    Accepted,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceMapping {
    pub source: TraceSource,
    pub attribute: TraceAttribute,
    pub pointer: JsonPointer,
}

fn relative_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains(['?', '#', '{', '}', '\\'])
        && !path.chars().any(char::is_control)
}

fn overlaps(left: &JsonPointer, right: &JsonPointer) -> bool {
    left == right
        || left
            .as_str()
            .strip_prefix(right.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .as_str()
            .strip_prefix(left.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

impl AgentIo {
    pub fn validate(&self) -> Result<(), InvalidAgentIo> {
        if self.version != 1 {
            return Err(InvalidAgentIo::Version);
        }
        if self.connection.len() > 64
            || !self
                .connection
                .starts_with(|value: char| value.is_ascii_alphabetic())
            || !self
                .connection
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-'))
        {
            return Err(InvalidAgentIo::Connection);
        }
        if !relative_path(&self.submit.path) {
            return Err(InvalidAgentIo::RequestPath);
        }
        if !(200..=299).contains(&self.submit.accepted_status) {
            return Err(InvalidAgentIo::AcceptedStatus);
        }
        if !self
            .input
            .iter()
            .any(|binding| binding.source == InputSource::CaseInput)
        {
            return Err(InvalidAgentIo::MissingInput);
        }
        if self.input.iter().enumerate().any(|(index, binding)| {
            self.input[..index]
                .iter()
                .any(|other| overlaps(&binding.target, &other.target))
        }) {
            return Err(InvalidAgentIo::OverlappingInputTargets);
        }
        let body = Value::Object(self.submit.json.clone());
        if self.input.iter().any(|binding| {
            binding.target.resolve(&body).is_none_or(|value| {
                value.is_object() || value.as_array().is_some_and(|array| !array.is_empty())
            })
        }) {
            return Err(InvalidAgentIo::InputTarget);
        }
        if let Completion::Poll(poll) = &self.completion {
            poll.validate()?;
        }
        Ok(())
    }
}

impl PollCompletion {
    pub fn validate(&self) -> Result<(), InvalidAgentIo> {
        if self.path_template.matches("{id}").count() != 1
            || !relative_path(&self.path_template.replace("{id}", "id"))
        {
            return Err(InvalidAgentIo::PollPath);
        }
        let success = self.success.iter().collect::<BTreeSet<_>>();
        let failure = self.failure.iter().collect::<BTreeSet<_>>();
        if success.is_empty()
            || failure.is_empty()
            || success.len() != self.success.len()
            || failure.len() != self.failure.len()
            || !success.is_disjoint(&failure)
            || success.union(&failure).any(|state| state.trim().is_empty())
        {
            return Err(InvalidAgentIo::PollStates);
        }
        if !(250..=60_000).contains(&self.interval_ms)
            || !(1000..=3_600_000).contains(&self.timeout_ms)
            || self.interval_ms > self.timeout_ms
        {
            return Err(InvalidAgentIo::PollTiming);
        }
        if self
            .require_item
            .as_ref()
            .is_some_and(|item| item.matches.is_empty())
        {
            return Err(InvalidAgentIo::EmptyMatch);
        }
        Ok(())
    }
}
