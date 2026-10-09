use base64::{Engine, engine::general_purpose::URL_SAFE};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionId {
    pub source: String,
    pub team_id: String,
    pub trace_id: String,
    pub trace_ref: String,
}

impl ExecutionId {
    pub fn encode(&self) -> String {
        let parts =
            [&self.source, &self.team_id, &self.trace_id, &self.trace_ref].map(|part| quoted(part));
        URL_SAFE.encode(format!("[{}]", parts.join(", ")))
    }

    pub fn selection_key(&self) -> String {
        format!(
            "{}\0{}\0{}",
            self.source,
            self.team_id,
            if self.trace_ref.is_empty() {
                &self.trace_id
            } else {
                &self.trace_ref
            },
        )
    }

    pub fn decode(value: &str) -> Option<Self> {
        if !value.is_ascii() {
            return None;
        }
        let mut encoded = String::new();
        let mut padding = 0;
        let mut terminated = false;
        for character in value.chars() {
            if character == '=' {
                padding += 1;
                if encoded.len() % 4 >= 2 && encoded.len() % 4 + padding >= 4 {
                    terminated = true;
                    break;
                }
                continue;
            }
            let character = match character {
                '-' => '+',
                '_' => '/',
                'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => character,
                _ => continue,
            };
            padding = 0;
            encoded.push(character);
        }
        if !terminated && !encoded.len().is_multiple_of(4) {
            return None;
        }
        let engine = base64::engine::general_purpose::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            base64::engine::general_purpose::GeneralPurposeConfig::new()
                .with_decode_allow_trailing_bits(true)
                .with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
        );
        let decoded = engine.decode(encoded).ok()?;
        let parts = serde_json::from_slice::<Vec<String>>(&decoded).ok()?;
        let (source, team_id, trace_id, trace_ref) = match parts.as_slice() {
            [source, team_id, trace_id] => (source, team_id, trace_id, String::new()),
            [source, team_id, trace_id, trace_ref] => {
                (source, team_id, trace_id, trace_ref.clone())
            }
            _ => return None,
        };
        Some(Self {
            source: source.clone(),
            team_id: team_id.clone(),
            trace_id: trace_id.clone(),
            trace_ref,
        })
    }
}

fn quoted(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            ' '..='~' => output.push(character),
            character => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    output.push('"');
    output
}
