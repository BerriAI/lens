use serde_json::{Map, Value, json};

const DROPPED: [&str; 13] = [
    "$schema",
    "default",
    "definitions",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "format",
    "maxItems",
    "maxLength",
    "maximum",
    "minItems",
    "minLength",
    "minimum",
    "pattern",
];

pub(crate) fn strict(schema: &Value) -> Option<Value> {
    let definitions = schema.get("definitions").and_then(Value::as_object);
    normalize(schema, definitions, &mut Vec::new())
}

pub(crate) fn name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

fn normalize<'a>(
    value: &'a Value,
    definitions: Option<&'a Map<String, Value>>,
    resolving: &mut Vec<&'a str>,
) -> Option<Value> {
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("$ref").and_then(Value::as_str) {
                let name = reference.strip_prefix("#/definitions/")?;
                if resolving.contains(&name) {
                    return None;
                }
                resolving.push(name);
                let resolved = normalize(definitions?.get(name)?, definitions, resolving);
                resolving.pop();
                return resolved;
            }
            let mut normalized = Map::new();
            for (key, value) in fields {
                if DROPPED.contains(&key.as_str()) {
                    continue;
                }
                let value = match (key.as_str(), value) {
                    ("properties", Value::Object(properties)) => Value::Object(
                        properties
                            .iter()
                            .map(|(name, property)| {
                                Some((name.clone(), normalize(property, definitions, resolving)?))
                            })
                            .collect::<Option<_>>()?,
                    ),
                    ("const", constant) => {
                        normalized.insert("enum".into(), json!([constant]));
                        continue;
                    }
                    ("enum" | "required", listed) => listed.clone(),
                    (_, value) => normalize(value, definitions, resolving)?,
                };
                normalized.insert(key.clone(), value);
            }
            if let Some(Value::Object(properties)) = normalized.get("properties") {
                let mut required: Vec<_> = properties.keys().cloned().collect();
                required.sort();
                normalized.insert("required".into(), json!(required));
                normalized.insert("additionalProperties".into(), Value::Bool(false));
            }
            Some(Value::Object(normalized))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| normalize(item, definitions, resolving))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        other => Some(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn every_object_is_closed_and_fully_required_without_references() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "minLength": 3},
                "kind": {"type": "string", "default": "issue", "enum": ["issue", "pattern"]},
                "action": {"type": "string", "const": "python"},
                "evidence": {"type": "array", "minItems": 1, "items": {"$ref": "#/definitions/Evidence"}},
                "brief": {"anyOf": [{"$ref": "#/definitions/Evidence"}, {"type": "null"}]}
            },
            "required": ["title"],
            "definitions": {"Evidence": {
                "type": "object",
                "properties": {"quote": {"type": "string", "minLength": 1}, "default": {"type": "integer", "minimum": 0}},
                "required": ["quote"]
            }}
        });
        let evidence = json!({
            "type": "object",
            "properties": {"quote": {"type": "string"}, "default": {"type": "integer"}},
            "required": ["default", "quote"],
            "additionalProperties": false
        });
        assert_eq!(
            strict(&schema).unwrap(),
            json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "kind": {"type": "string", "enum": ["issue", "pattern"]},
                    "action": {"type": "string", "enum": ["python"]},
                    "evidence": {"type": "array", "items": evidence},
                    "brief": {"anyOf": [evidence, {"type": "null"}]}
                },
                "required": ["action", "brief", "evidence", "kind", "title"],
                "additionalProperties": false
            })
        );
    }

    #[rstest]
    #[case::recursive(json!({"$ref": "#/definitions/Node", "definitions": {"Node": {"type": "object", "properties": {"next": {"$ref": "#/definitions/Node"}}}}}))]
    #[case::unknown_reference(json!({"type": "object", "properties": {"item": {"$ref": "#/definitions/Missing"}}}))]
    #[case::external_reference(json!({"type": "object", "properties": {"item": {"$ref": "https://example.com/schema"}}}))]
    fn schemas_that_cannot_be_inlined_are_not_sent(#[case] schema: Value) {
        assert_eq!(strict(&schema), None);
    }

    #[rstest]
    fn names_are_reduced_to_the_provider_alphabet() {
        assert_eq!(
            name("PythonAgentTurn[Findings]"),
            "PythonAgentTurn_Findings_"
        );
        assert_eq!(name(&"a".repeat(80)).len(), 64);
    }
}
