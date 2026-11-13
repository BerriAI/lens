use crate::{Error, wire};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use tokio::io::AsyncReadExt;

#[derive(Serialize, Deserialize)]
pub struct Turn {
    pub response: String,
    pub tool_results: Vec<String>,
    pub validation_error: String,
}

pub struct Journal {
    directory: tempfile::TempDir,
    pub turns: Vec<usize>,
}

struct Excerpt {
    start: usize,
    end: usize,
    characters: usize,
    text: String,
}

impl Excerpt {
    fn append(&mut self, text: &str) -> Result<(), Error> {
        let length = text.chars().count();
        let start = self.start.saturating_sub(self.characters);
        let end = self.end.saturating_sub(self.characters).min(length);
        if start < end {
            for character in text.chars().skip(start).take(end - start) {
                self.text.push(character);
            }
        }
        self.characters += length;
        Ok(())
    }

    async fn append_file(&mut self, path: &Path) -> Result<(), Error> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut buffer = [0u8; 64 * 1024];
        let mut pending = Vec::new();
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                return if pending.is_empty() {
                    Ok(())
                } else {
                    Err(Error::InvalidRequest)
                };
            }
            pending.extend_from_slice(&buffer[..count]);
            let valid = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => return Err(Error::InvalidRequest),
            };
            self.append(
                std::str::from_utf8(&pending[..valid]).map_err(|_| Error::InvalidRequest)?,
            )?;
            pending.drain(..valid);
        }
    }
}

impl Journal {
    pub async fn new(initial: &Value) -> Result<Self, Error> {
        let directory = tempfile::Builder::new().prefix("lens-journal-").tempdir()?;
        let bytes = serde_json::to_vec(initial)?;
        tokio::fs::write(directory.path().join("initial"), &bytes).await?;
        Ok(Self {
            directory,
            turns: Vec::new(),
        })
    }

    pub async fn push(&mut self, turn: &Turn) -> Result<(), Error> {
        let encoded = serde_json::to_string(turn)?;
        tokio::fs::write(
            self.directory.path().join(self.turns.len().to_string()),
            encoded.as_bytes(),
        )
        .await?;
        self.turns.push(encoded.chars().count());
        Ok(())
    }

    pub async fn reply(&self, request: &wire::EvidenceRequest) -> Result<Value, Error> {
        self.reply_at(request, self.turns.len()).await
    }

    pub async fn page(
        &self,
        request: &wire::EvidenceRequest,
        characters: u64,
    ) -> Result<Value, Error> {
        if request.turn_end == Some(0) && !request.include_initial {
            return self.reply(request).await;
        }
        let mut bounded = request.clone();
        let snapshot = request
            .turn_end
            .unwrap_or(self.turns.len() as u64)
            .min(self.turns.len() as u64);
        bounded.turn_end = Some(snapshot);
        bounded.char_end = Some(
            request
                .char_end
                .unwrap_or(u64::MAX)
                .min(request.char_start.saturating_add(characters.max(1))),
        );
        let mut reply = self.reply_at(&bounded, snapshot as usize).await?;
        let Some(total) = reply["characters"].as_u64() else {
            return Ok(reply);
        };
        if request.char_start > total {
            return Ok(
                json!({"request": request, "error": "Choose a valid journal character range"}),
            );
        }
        let end = bounded.char_end.unwrap_or(total).min(total);
        bounded.char_end = Some(end);
        if request.char_start == 0 && end == total {
            let mut full = bounded.clone();
            full.char_end = None;
            reply = self.reply_at(&full, snapshot as usize).await?;
        }
        reply["request"] = serde_json::to_value(&bounded)?;
        reply["characters"] = json!(total);
        reply["next_request"] = if end < total.min(request.char_end.unwrap_or(u64::MAX)) {
            let mut next = bounded;
            next.char_start = end;
            next.char_end = request.char_end;
            serde_json::to_value(next)?
        } else {
            Value::Null
        };
        Ok(reply)
    }

    async fn reply_at(
        &self,
        request: &wire::EvidenceRequest,
        snapshot: usize,
    ) -> Result<Value, Error> {
        let start = request.turn_start as usize;
        let end = request
            .turn_end
            .map(|n| n as usize)
            .unwrap_or(self.turns.len())
            .min(self.turns.len());
        if start > end || request.char_end.is_some_and(|end| end < request.char_start) {
            return Ok(
                json!({"request": request, "error": "Choose a valid journal turn and character range"}),
            );
        }
        if request.char_start != 0 || request.char_end.is_some() {
            return self.excerpt(request, start, end, snapshot).await;
        }
        let mut turns = Vec::<Value>::new();
        for index in start..end {
            let path = self.directory.path().join(index.to_string());
            turns.push(serde_json::from_slice(&tokio::fs::read(path).await?)?);
        }
        let initial: Value = if request.include_initial {
            serde_json::from_slice(&tokio::fs::read(self.directory.path().join("initial")).await?)?
        } else {
            Value::Null
        };
        let mut normalized = request.clone();
        normalized.char_start = 0;
        normalized.char_end = None;
        let reply = json!({"request": normalized, "total_turns": snapshot, "initial_context": initial, "turns": turns, "turn_characters": &self.turns[..snapshot]});
        Ok(reply)
    }

    async fn excerpt(
        &self,
        request: &wire::EvidenceRequest,
        start: usize,
        end: usize,
        snapshot: usize,
    ) -> Result<Value, Error> {
        let mut normalized = request.clone();
        normalized.char_start = 0;
        normalized.char_end = None;
        let document = json!({"request": normalized, "total_turns": snapshot, "initial_context": null, "turns": [], "turn_characters": &self.turns[..snapshot]});
        let mut excerpt = Excerpt {
            start: request.char_start as usize,
            end: request
                .char_end
                .map(|value| value as usize)
                .unwrap_or(usize::MAX),
            characters: 0,
            text: String::new(),
        };
        excerpt.append("{")?;
        for (index, (key, value)) in document
            .as_object()
            .ok_or(Error::InvalidRequest)?
            .iter()
            .enumerate()
        {
            if index != 0 {
                excerpt.append(",")?;
            }
            excerpt.append(&serde_json::to_string(key)?)?;
            excerpt.append(":")?;
            match key.as_str() {
                "initial_context" if request.include_initial => {
                    excerpt
                        .append_file(&self.directory.path().join("initial"))
                        .await?;
                }
                "turns" => {
                    excerpt.append("[")?;
                    for turn in start..end {
                        if turn != start {
                            excerpt.append(",")?;
                        }
                        excerpt
                            .append_file(&self.directory.path().join(turn.to_string()))
                            .await?;
                    }
                    excerpt.append("]")?;
                }
                _ => excerpt.append(&serde_json::to_string(value)?)?,
            }
        }
        excerpt.append("}")?;
        Ok(
            json!({"request": request, "total_turns": snapshot, "excerpt": excerpt.text, "characters": excerpt.characters}),
        )
    }

    pub fn reference(&self, request: &wire::EvidenceRequest) -> Option<String> {
        if request.action != wire::EvidenceRequestAction::History
            || request.char_end.is_some_and(|end| end < request.char_start)
            || request.turn_start as usize > self.turns.len()
            || request.turn_end.is_some_and(|n| n < request.turn_start)
        {
            return None;
        }
        let mut request = request.clone();
        request.turn_end = Some(
            request
                .turn_end
                .unwrap_or(self.turns.len() as u64)
                .min(self.turns.len() as u64),
        );
        Some(json!({"kind": "history_reference", "request": request, "recorded_turns": self.turns.len()}).to_string())
    }
}
