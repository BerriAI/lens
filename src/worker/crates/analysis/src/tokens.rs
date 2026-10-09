use crate::Error;
use litellm_model_catalog::ModelEntry;
use litellm_token_counter::{TokenCounter, Tokenizer};

pub(crate) fn counter(model: &str, entry: Option<&ModelEntry>) -> Result<TokenCounter, Error> {
    let provider = entry.and_then(|entry| entry.info().litellm_provider.as_deref());
    let encoder: Box<dyn Tokenizer> =
        if provider == Some("anthropic") && !model.contains("claude-3") {
            Box::new(
                litellm_token_counter::huggingface::HuggingFaceTokenizer::from_json(include_str!(
                    "../data/anthropic_tokenizer.json"
                ))
                .map_err(litellm_token_counter::Error::from)?,
            )
        } else {
            let model = if provider == Some("openai") {
                model
            } else {
                "gpt-3.5-turbo"
            };
            let encoding = if model.contains("gpt-4o") {
                "o200k_base"
            } else {
                litellm_token_counter::tiktoken::encoding_for_model(model).unwrap_or("cl100k_base")
            };
            Box::new(
                litellm_token_counter::tiktoken::TiktokenTokenizer::from_name(encoding)
                    .map_err(litellm_token_counter::Error::from)?,
            )
        };
    Ok(TokenCounter::new(Extrapolated {
        encoder,
        maximum: 4_000_000,
    }))
}

struct Extrapolated {
    encoder: Box<dyn Tokenizer>,
    maximum: usize,
}

impl Tokenizer for Extrapolated {
    fn count_tokens(&self, text: &str) -> Result<usize, litellm_token_counter::Error> {
        let chars = text.chars().count();
        if chars <= self.maximum {
            return self.encoder.count_tokens(text);
        }
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
            .collect();
        let count = 16.min(self.maximum);
        let width = self.maximum / count;
        let last = chars - width;
        let tokens = (0..count)
            .map(|index| {
                let start = last * index / (count - 1).max(1);
                self.encoder
                    .count_tokens(&text[boundaries[start]..boundaries[start + width]])
            })
            .sum::<Result<usize, _>>()?;
        Ok((tokens as f64 * chars as f64 / (count * width) as f64).round_ties_even() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::sync::{Arc, Mutex};

    struct Recording {
        samples: Arc<Mutex<Vec<String>>>,
    }
    impl Tokenizer for Recording {
        fn count_tokens(&self, text: &str) -> Result<usize, litellm_token_counter::Error> {
            self.samples.lock().unwrap().push(text.into());
            Ok(text.chars().filter(|character| *character == 'x').count())
        }
    }

    #[rstest]
    #[case::below("xéx",4,vec!["xéx"],2)]
    #[case::at_limit("xéxé",4,vec!["xéxé"],2)]
    #[case::unicode("xééxééxééxéé",4,vec!["x","x","é","é"],6)]
    #[case::one_sample("xéé",1,vec!["x"],3)]
    #[case::round_down_even("xéxxé",2,vec!["x","é"],2)]
    #[case::round_up_even("xéxxéxx",2,vec!["x","x"],7)]
    fn long_text_uses_evenly_spaced_unicode_samples(
        #[case] text: &str,
        #[case] maximum: usize,
        #[case] expected: Vec<&str>,
        #[case] count: usize,
    ) {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let counter = Extrapolated {
            encoder: Box::new(Recording {
                samples: samples.clone(),
            }),
            maximum,
        };
        assert_eq!(counter.count_tokens(text).unwrap(), count);
        assert_eq!(*samples.lock().unwrap(), expected);
    }

    #[rstest]
    #[case::full_windows(32,vec!["xx";16])]
    #[case::floored_windows(33,vec!["xx";16])]
    fn long_text_sample_count_stays_bounded(#[case] maximum: usize, #[case] expected: Vec<&str>) {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let counter = Extrapolated {
            encoder: Box::new(Recording {
                samples: samples.clone(),
            }),
            maximum,
        };
        assert_eq!(counter.count_tokens(&"x".repeat(96)).unwrap(), 96);
        assert_eq!(*samples.lock().unwrap(), expected);
    }

    #[rstest]
    #[case::known_openai("gpt-4o", "openai", "o200k_base")]
    #[case::unknown_openai("fixture", "openai", "cl100k_base")]
    #[case::claude3("claude-3-fixture", "anthropic", "cl100k_base")]
    #[case::other("fixture", "custom", "cl100k_base")]
    fn tiktoken_selection_matches_python(
        #[case] model: &str,
        #[case] provider: &str,
        #[case] encoding: &str,
    ) {
        let body =
            serde_json::to_vec(&serde_json::json!({model:{"litellm_provider":provider}})).unwrap();
        let catalog = litellm_model_catalog::Catalog::parse(&body, Default::default()).unwrap();
        let actual = counter(model, Some(catalog.lookup(model).unwrap().entry)).unwrap();
        let expected = TokenCounter::from_tiktoken(encoding).unwrap();
        let text = "The cost of 東京 is €12.50; investigate this trace.";
        assert_eq!(
            actual.count_text(text).unwrap(),
            expected.count_text(text).unwrap()
        );
    }

    #[rstest]
    fn direct_anthropic_models_use_bundled_tokenizer() {
        let body = br#"{"fixture":{"litellm_provider":"anthropic"}}"#;
        let catalog = litellm_model_catalog::Catalog::parse(body, Default::default()).unwrap();
        let actual = counter("fixture", Some(catalog.lookup("fixture").unwrap().entry)).unwrap();
        let expected = TokenCounter::new(
            litellm_token_counter::huggingface::HuggingFaceTokenizer::from_json(include_str!(
                "../data/anthropic_tokenizer.json"
            ))
            .unwrap(),
        );
        let text = "The cost of 東京 is €12.50; investigate this trace.";
        assert_eq!(
            actual.count_text(text).unwrap(),
            expected.count_text(text).unwrap()
        );
        let fallback = counter("anthropic/fixture", None).unwrap();
        assert_eq!(
            fallback.count_text(text).unwrap(),
            TokenCounter::from_tiktoken("cl100k_base")
                .unwrap()
                .count_text(text)
                .unwrap()
        );
    }
}
