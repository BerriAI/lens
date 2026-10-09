use crate::{eval, worker};
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

pub fn worker_contract() -> Value {
    let mut generator = SchemaSettings::draft07().into_generator();
    let _ = generator.subschema_for::<worker::Activity>();
    let _ = generator.subschema_for::<worker::AgentTestCase>();
    let _ = generator.subschema_for::<worker::Candidate>();
    let _ = generator.subschema_for::<worker::CatalogEntry>();
    let _ = generator.subschema_for::<worker::Check>();
    let _ = generator.subschema_for::<worker::Checkpoint>();
    let _ = generator.subschema_for::<worker::Claim>();
    let _ = generator.subschema_for::<worker::Clusters>();
    let _ = generator.subschema_for::<worker::Coverage>();
    let _ = generator.subschema_for::<worker::Evidence>();
    let _ = generator.subschema_for::<worker::EvidenceReply>();
    let _ = generator.subschema_for::<worker::EvidenceRequest>();
    let _ = generator.subschema_for::<worker::Execution>();
    let _ = generator.subschema_for::<worker::ExecutionContent>();
    let _ = generator.subschema_for::<worker::Extraction>();
    let _ = generator.subschema_for::<worker::Finding>();
    let _ = generator.subschema_for::<worker::FindingDraft>();
    let _ = generator.subschema_for::<worker::FindingGroup>();
    let _ = generator.subschema_for::<worker::FindingGroups>();
    let _ = generator.subschema_for::<worker::Findings>();
    let _ = generator.subschema_for::<worker::InFlight>();
    let _ = generator.subschema_for::<worker::IssueBrief>();
    let _ = generator.subschema_for::<worker::Job>();
    let _ = generator.subschema_for::<worker::LensSettings>();
    let _ = generator.subschema_for::<worker::MetadataFilter>();
    let _ = generator.subschema_for::<worker::ModelMessage>();
    let _ = generator.subschema_for::<worker::ModelRequest>();
    let _ = generator.subschema_for::<worker::ModelResult>();
    let _ = generator.subschema_for::<worker::Observation>();
    let _ = generator.subschema_for::<worker::Progress>();
    let _ = generator.subschema_for::<worker::PythonAgentTurnExtraction>();
    let _ = generator.subschema_for::<worker::PythonAgentTurnFindings>();
    let _ = generator.subschema_for::<worker::PythonRequest>();
    let _ = generator.subschema_for::<worker::Result>();
    let _ = generator.subschema_for::<worker::Review>();
    let _ = generator.subschema_for::<worker::ReviewIndex>();
    let _ = generator.subschema_for::<worker::ReviewRecord>();
    let _ = generator.subschema_for::<worker::ReviewSpan>();
    let _ = generator.subschema_for::<worker::ReviewVerdict>();
    let _ = generator.subschema_for::<worker::ReviewVersion>();
    let _ = generator.subschema_for::<worker::RunAssessment>();
    let _ = generator.subschema_for::<worker::Sample>();
    let _ = generator.subschema_for::<worker::Step>();
    let _ = generator.subschema_for::<worker::ToolCount>();
    let _ = generator.subschema_for::<worker::TracePart>();
    let mut schema = normalize(
        json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "definitions": generator.take_definitions(false),
            "x-lens-protocol-version": worker::PROTOCOL_VERSION,
        }),
        false,
    );
    schema.sort_all_objects();
    schema
}

pub fn eval_contract() -> Value {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let _ = generator.subschema_for::<eval::ApiError>();
    let _ = generator.subschema_for::<eval::CaseDiff>();
    let _ = generator.subschema_for::<eval::CaseResult>();
    let _ = generator.subschema_for::<eval::CreateEvalRun>();
    let _ = generator.subschema_for::<eval::EvalRun>();
    let _ = generator.subschema_for::<eval::GateResult>();
    let _ = generator.subschema_for::<eval::ResolvedDataset>();
    let _ = generator.subschema_for::<eval::RunCase>();
    let _ = generator.subschema_for::<eval::Summary>();
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "LensEvalContract",
        "type": "object",
        "$defs": generator.take_definitions(false),
    });
    schema.sort_all_objects();
    schema
}

fn normalize(value: Value, names: bool) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|value| normalize(value, false))
                .collect(),
        ),
        Value::Object(properties) => {
            let mut normalized: Map<String, Value> = properties
                .into_iter()
                .filter_map(|(name, value)| {
                    if !names
                        && (name == "title"
                            || (name == "default" && value.is_null())
                            || (name == "format"
                                && matches!(
                                    value.as_str(),
                                    Some(
                                        "int64"
                                            | "int32"
                                            | "int"
                                            | "uint64"
                                            | "uint32"
                                            | "uint"
                                            | "double"
                                            | "float"
                                    )
                                )))
                    {
                        return None;
                    }
                    let is_names = !names
                        && matches!(
                            name.as_str(),
                            "properties" | "definitions" | "patternProperties"
                        );
                    let value = normalize(value, is_names);
                    let name = if !names && name == "prefixItems" {
                        "items".to_owned()
                    } else {
                        name
                    };
                    Some((name, value))
                })
                .collect();
            if !names
                && let Some(Value::Array(types)) = normalized.get("type")
                && types.len() == 2
                && types.contains(&json!("null"))
            {
                let concrete = types.iter().find(|value| *value != &json!("null")).cloned();
                if let Some(concrete) = concrete {
                    let mut branch = normalized.clone();
                    branch.insert("type".into(), concrete);
                    if let Some(Value::Array(values)) = branch.get_mut("enum") {
                        values.retain(|value| !value.is_null());
                    }
                    normalized.clear();
                    for name in ["default", "description"] {
                        if let Some(value) = branch.remove(name) {
                            normalized.insert(name.into(), value);
                        }
                    }
                    normalized.insert("anyOf".into(), json!([branch, {"type":"null"}]));
                }
            }
            Value::Object(normalized)
        }
        other => other,
    }
}
