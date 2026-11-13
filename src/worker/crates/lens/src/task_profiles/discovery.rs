use std::{collections::BTreeMap, future::Future};

use lens_contract::profiles::{Capability, DiscoveryPoint, TaskDefinition, TaxonomyModels};
use serde::Deserialize;

use crate::error::DiscoveryError;

pub const MAX_TASKS: usize = 12;
const DISTILL_BATCH: usize = 25;
const EXAMPLES_PER_CLUSTER: usize = 12;
const KEPT_EXAMPLES: usize = 3;
const INPUT_CHARACTERS: usize = 1_500;
const FALLBACK_CHARACTERS: usize = 300;

const DISTILL_PROMPT: &str = "Each numbered item is a request someone sent to an AI agent.
For every item write one short sentence, at most 15 words, saying what kind of work it asks for.
Leave out product, company, person, repository, file and other proper names.

Reply with only a JSON object {\"intents\": [...]} holding one string per item, in the same order.

Items:
";

const SYNTHESIS_PROMPT: &str = "You are naming one group of requests that people sent to an AI agent.
Read the numbered requests and describe the kind of work they share.

Rules:
- Name the kind of work, such as \"Summarize documents\" or \"Debug failing tests\", in at most 6 words.
- Never use product, company, person, repository or file names. Describe the work, not the subject it was done for.
- inclusion_criteria says what a request must ask for to belong. exclusion_criteria names the nearest kinds of work that do not belong.
- required_capabilities lists only what every request needs from a model, chosen from function_calling, vision, reasoning and structured_output. Leave it empty when unsure.
- example_indices lists up to 3 request numbers that best represent the group.

Reply with only a JSON object. name, description, inclusion_criteria and exclusion_criteria are strings of one or two sentences, required_capabilities is an array of strings, and example_indices is an array of integers.

Requests:
";

pub trait Language: Send + Sync {
    fn complete(
        &self,
        model: &str,
        prompt: String,
    ) -> impl Future<Output = Result<String, DiscoveryError>> + Send;
}

pub trait Embeddings: Send + Sync {
    fn embed(
        &self,
        model: &str,
        texts: &[String],
    ) -> impl Future<Output = Result<Vec<Vec<f32>>, DiscoveryError>> + Send;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub team_id: String,
    pub api_key_hash: String,
    pub trace_id: String,
    pub text: String,
}

fn excerpt(text: &str, characters: usize) -> String {
    text.chars().take(characters).collect()
}

fn numbered<'a>(texts: impl Iterator<Item = &'a str>) -> String {
    texts
        .enumerate()
        .map(|(index, text)| {
            format!(
                "{index}. {}",
                serde_json::to_string(&excerpt(text, INPUT_CHARACTERS)).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn json_object(reply: &str) -> &str {
    match (reply.find('{'), reply.rfind('}')) {
        (Some(start), Some(end)) if end > start => &reply[start..=end],
        _ => reply,
    }
}

#[derive(Deserialize)]
struct Intents {
    intents: Vec<String>,
}

/// One-line intents, after LLooM's distillation step, so domain vocabulary does not drive the
/// clusters. A batch whose reply does not parse keeps its own excerpts instead.
async fn distill(
    language: &impl Language,
    model: &str,
    texts: &[String],
) -> Result<Vec<String>, DiscoveryError> {
    let mut intents = Vec::with_capacity(texts.len());
    for batch in texts.chunks(DISTILL_BATCH) {
        let reply = language
            .complete(
                model,
                format!(
                    "{DISTILL_PROMPT}{}",
                    numbered(batch.iter().map(String::as_str))
                ),
            )
            .await?;
        let parsed = serde_json::from_str::<Intents>(json_object(&reply))
            .ok()
            .map(|parsed| parsed.intents)
            .filter(|parsed| {
                parsed.len() == batch.len() && parsed.iter().all(|intent| !intent.trim().is_empty())
            });
        intents.extend(parsed.unwrap_or_else(|| {
            batch
                .iter()
                .map(|text| excerpt(text, FALLBACK_CHARACTERS))
                .collect()
        }));
    }
    Ok(intents)
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Text {
    #[default]
    Missing,
    One(String),
    Many(Vec<String>),
}

impl Text {
    fn sentence(self) -> String {
        match self {
            Self::Missing => String::new(),
            Self::One(text) => text.trim().to_owned(),
            Self::Many(items) => items
                .iter()
                .map(|item| item.trim().trim_end_matches('.'))
                .filter(|item| !item.is_empty())
                .map(|item| format!("{item}."))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

#[derive(Deserialize)]
struct Synthesis {
    name: String,
    #[serde(default)]
    description: Text,
    #[serde(default)]
    inclusion_criteria: Text,
    #[serde(default)]
    exclusion_criteria: Text,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    example_indices: Vec<usize>,
}

fn capability(name: &str) -> Option<Capability> {
    match name.trim() {
        "function_calling" => Some(Capability::FunctionCalling),
        "vision" => Some(Capability::Vision),
        "reasoning" => Some(Capability::Reasoning),
        "structured_output" => Some(Capability::StructuredOutput),
        _ => None,
    }
}

fn clipped(text: String, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn definition(reply: &str, task_id: String, members: &[&Sample]) -> TaskDefinition {
    let fallback_examples = || {
        members
            .iter()
            .take(KEPT_EXAMPLES)
            .map(|sample| sample.trace_id.clone())
            .collect()
    };
    let Some(synthesis) = serde_json::from_str::<Synthesis>(json_object(reply))
        .ok()
        .filter(|synthesis| (1..=80).contains(&synthesis.name.trim().chars().count()))
    else {
        return TaskDefinition {
            task_id,
            name: "Unnamed group".into(),
            description: "The synthesis model returned no usable definition. Review and rename before freezing.".into(),
            inclusion_criteria: String::new(),
            exclusion_criteria: String::new(),
            required_capabilities: Vec::new(),
            example_trace_ids: fallback_examples(),
        };
    };
    let examples: Vec<String> = synthesis
        .example_indices
        .iter()
        .filter_map(|index| members.get(*index))
        .take(KEPT_EXAMPLES)
        .map(|sample| sample.trace_id.clone())
        .collect();
    let mut capabilities: Vec<Capability> = synthesis
        .required_capabilities
        .iter()
        .filter_map(|name| capability(name))
        .collect();
    capabilities.sort();
    capabilities.dedup();
    TaskDefinition {
        task_id,
        name: synthesis.name.trim().to_owned(),
        description: clipped(synthesis.description.sentence(), 600),
        inclusion_criteria: clipped(synthesis.inclusion_criteria.sentence(), 600),
        exclusion_criteria: clipped(synthesis.exclusion_criteria.sentence(), 600),
        required_capabilities: capabilities,
        example_trace_ids: if examples.is_empty() {
            fallback_examples()
        } else {
            examples
        },
    }
}

/// Distill, embed, cluster, then name each of the largest groups. Paid stages run in order and a
/// provider failure stops the run, which the caller records on the taxonomy.
pub async fn discover(
    language: &impl Language,
    embeddings: &impl Embeddings,
    models: &TaxonomyModels,
    samples: &[Sample],
    mut task_id: impl FnMut() -> String,
) -> Result<(Vec<TaskDefinition>, Vec<DiscoveryPoint>), DiscoveryError> {
    let texts: Vec<String> = samples.iter().map(|sample| sample.text.clone()).collect();
    let intents = distill(language, &models.distill, &texts).await?;
    let vectors = embeddings.embed(&models.embedding, &intents).await?;
    if vectors.len() != samples.len() {
        return Err(DiscoveryError::InvalidReply("embedding"));
    }
    let layout = tokio::task::spawn_blocking(move || lens_profiles::clustering::cluster(&vectors))
        .await
        .map_err(|_| DiscoveryError::Clustering)?
        .map_err(|_| DiscoveryError::Clustering)?;
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, label) in layout.labels.iter().enumerate() {
        if let Some(label) = label {
            groups.entry(*label).or_default().push(index);
        }
    }
    let mut ranked: Vec<(usize, Vec<usize>)> = groups.into_iter().collect();
    ranked.sort_by(|(left_label, left), (right_label, right)| {
        right
            .len()
            .cmp(&left.len())
            .then(left_label.cmp(right_label))
    });
    ranked.truncate(MAX_TASKS);
    let mut tasks = Vec::with_capacity(ranked.len());
    let mut assigned: BTreeMap<usize, String> = BTreeMap::new();
    for (_, members) in &ranked {
        let shown: Vec<&Sample> = members
            .iter()
            .take(EXAMPLES_PER_CLUSTER)
            .map(|index| &samples[*index])
            .collect();
        let reply = language
            .complete(
                &models.synthesis,
                format!(
                    "{SYNTHESIS_PROMPT}{}",
                    numbered(shown.iter().map(|sample| sample.text.as_str()))
                ),
            )
            .await?;
        let task = definition(&reply, task_id(), &shown);
        for index in members {
            assigned.insert(*index, task.task_id.clone());
        }
        tasks.push(task);
    }
    let points = samples
        .iter()
        .zip(&layout.points)
        .enumerate()
        .map(|(index, (sample, (x, y)))| DiscoveryPoint {
            team_id: sample.team_id.clone(),
            api_key_hash: sample.api_key_hash.clone(),
            trace_id: sample.trace_id.clone(),
            task_id: assigned.get(&index).cloned(),
            x: *x,
            y: *y,
        })
        .collect();
    Ok((tasks, points))
}

/// Discovery must finish inside its claim's lease; a stalled provider fails the job instead of
/// letting another replica start the same paid work.
pub async fn discover_within(
    budget: std::time::Duration,
    language: &impl Language,
    embeddings: &impl Embeddings,
    models: &TaxonomyModels,
    samples: &[Sample],
    task_id: impl FnMut() -> String,
) -> Result<(Vec<TaskDefinition>, Vec<DiscoveryPoint>), DiscoveryError> {
    tokio::time::timeout(
        budget,
        discover(language, embeddings, models, samples, task_id),
    )
    .await
    .map_err(|_| DiscoveryError::TimedOut)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn sample(trace_id: &str) -> Sample {
        Sample {
            team_id: "team".into(),
            api_key_hash: "key".into(),
            trace_id: trace_id.into(),
            text: format!("request {trace_id}"),
        }
    }

    struct Reply(&'static str);

    impl Language for Reply {
        async fn complete(&self, _: &str, _: String) -> Result<String, DiscoveryError> {
            Ok(self.0.into())
        }
    }

    #[rstest]
    #[case::bulleted(r#"{"name": " Debug failing tests ", "inclusion_criteria": ["Asks to fix a test", "Mentions a failure."], "required_capabilities": ["function_calling", "telepathy", "function_calling"], "example_indices": [1, 9]}"#,
        "Debug failing tests", "Asks to fix a test. Mentions a failure.", vec![Capability::FunctionCalling], vec!["b"])]
    #[case::wrapped_in_prose("Here you go: {\"name\": \"Write tests\"} thanks", "Write tests", "", vec![], vec!["a", "b"])]
    #[case::invalid("not json", "Unnamed group", "", vec![], vec!["a", "b"])]
    #[case::overlong_name(r#"{"name": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#, "Unnamed group", "", vec![], vec!["a", "b"])]
    fn synthesis_replies_become_bounded_definitions(
        #[case] reply: &str,
        #[case] name: &str,
        #[case] inclusion: &str,
        #[case] capabilities: Vec<Capability>,
        #[case] examples: Vec<&str>,
    ) {
        let members = [sample("a"), sample("b")];
        let refs: Vec<&Sample> = members.iter().collect();

        let task = definition(reply, "task-1".into(), &refs);

        assert_eq!(task.name, name);
        assert_eq!(task.inclusion_criteria, inclusion);
        assert_eq!(task.required_capabilities, capabilities);
        assert_eq!(task.example_trace_ids, examples);
    }

    #[rstest]
    #[tokio::test]
    async fn a_distillation_reply_with_the_wrong_count_keeps_the_excerpts() {
        let texts = vec![
            "fix the flaky login test".to_owned(),
            "explain this module".to_owned(),
        ];

        let intents = distill(&Reply(r#"{"intents": ["only one"]}"#), "distill", &texts)
            .await
            .expect("distill");

        assert_eq!(intents, texts);
    }

    #[rstest]
    #[tokio::test]
    async fn a_matching_distillation_reply_replaces_the_excerpts() {
        let texts = vec![
            "fix the flaky login test".to_owned(),
            "explain this module".to_owned(),
        ];

        let intents = distill(
            &Reply(r#"{"intents": ["Fix a failing test", "Explain code"]}"#),
            "distill",
            &texts,
        )
        .await
        .expect("distill");

        assert_eq!(intents, vec!["Fix a failing test", "Explain code"]);
    }

    #[derive(Default)]
    struct Recording {
        calls: std::sync::Mutex<Vec<(String, &'static str)>>,
    }

    impl Recording {
        fn calls(&self) -> Vec<(String, &'static str)> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl Language for Recording {
        async fn complete(&self, model: &str, prompt: String) -> Result<String, DiscoveryError> {
            let stage = if prompt.starts_with(DISTILL_PROMPT) { "distill" } else { "name" };
            self.calls.lock().expect("calls").push((model.to_owned(), stage));
            if stage == "name" {
                let name = if prompt.contains("invoice") { "Answer billing questions" } else { "Fix login problems" };
                return Ok(format!(r#"{{"name": "{name}", "required_capabilities": ["vision"], "example_indices": [0]}}"#));
            }
            let intents: Vec<&str> = prompt
                .lines()
                .filter(|line| line.contains(". \""))
                .map(|line| if line.contains("invoice") { "billing question" } else { "login problem" })
                .collect();
            Ok(serde_json::json!({ "intents": intents }).to_string())
        }
    }

    #[derive(Default)]
    struct Axes {
        models: std::sync::Mutex<Vec<String>>,
    }

    impl Embeddings for Axes {
        async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, DiscoveryError> {
            self.models.lock().expect("models").push(model.to_owned());
            Ok(texts
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let mut vector = vec![0.02 * (index % 7) as f32; 8];
                    vector[if text.contains("billing") { 0 } else { 4 }] = 1.0;
                    vector
                })
                .collect())
        }
    }

    #[rstest]
    #[tokio::test]
    async fn discovery_distills_embeds_clusters_and_names_with_the_models_it_was_given() {
        let samples: Vec<Sample> = (0..30)
            .map(|index| Sample {
                text: if index < 15 { format!("Why is invoice {index} wrong?") } else { format!("I cannot log in, attempt {index}") },
                ..sample(&format!("t{index}"))
            })
            .collect();
        let language = Recording::default();
        let embeddings = Axes::default();
        let models = TaxonomyModels {
            distill: "summarizer".into(),
            embedding: "qwen".into(),
            synthesis: "namer".into(),
            decision: "jev".into(),
        };
        let mut ids = ["task-a", "task-b"].into_iter();

        let (tasks, points) = discover(&language, &embeddings, &models, &samples, || ids.next().unwrap_or("extra").into())
            .await
            .expect("discovery");

        let names: std::collections::BTreeSet<&str> = tasks.iter().map(|task| task.name.as_str()).collect();
        let billing = points.iter().filter(|point| point.task_id == points[0].task_id).count();
        assert_eq!(names, ["Answer billing questions", "Fix login problems"].into());
        assert_eq!(tasks[0].required_capabilities, vec![Capability::Vision]);
        assert_eq!(points.len(), 30);
        assert_eq!(billing, 15);
        assert_ne!(points[0].task_id, points[29].task_id);
        assert_eq!(
            language.calls(),
            vec![
                ("summarizer".to_owned(), "distill"),
                ("summarizer".to_owned(), "distill"),
                ("namer".to_owned(), "name"),
                ("namer".to_owned(), "name"),
            ]
        );
        assert_eq!(*embeddings.models.lock().expect("models"), vec!["qwen".to_owned()]);
    }

    #[rstest]
    #[tokio::test]
    async fn an_embedding_reply_of_the_wrong_length_fails_discovery() {
        let samples: Vec<Sample> = (0..20).map(|index| sample(&index.to_string())).collect();

        let result = discover(&Reply(r#"{"intents": []}"#), &NoEmbeddings, &TaxonomyModels::default(), &samples, || "id".into()).await;

        assert_eq!(result, Err(DiscoveryError::InvalidReply("embedding")));
    }

    struct Failing;

    impl Language for Failing {
        async fn complete(&self, _: &str, _: String) -> Result<String, DiscoveryError> {
            Err(DiscoveryError::Provider("synthesis"))
        }
    }

    struct NoEmbeddings;

    impl Embeddings for NoEmbeddings {
        async fn embed(&self, _: &str, _: &[String]) -> Result<Vec<Vec<f32>>, DiscoveryError> {
            Ok(Vec::new())
        }
    }

    struct Stalled;

    impl Language for Stalled {
        async fn complete(&self, _: &str, _: String) -> Result<String, DiscoveryError> {
            std::future::pending().await
        }
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn a_stalled_provider_fails_discovery_within_its_budget() {
        let samples: Vec<Sample> = (0..20).map(|index| sample(&index.to_string())).collect();

        let result = discover_within(
            std::time::Duration::from_secs(60),
            &Stalled,
            &NoEmbeddings,
            &TaxonomyModels::default(),
            &samples,
            || "id".into(),
        )
        .await;

        assert_eq!(result, Err(DiscoveryError::TimedOut));
    }

    #[rstest]
    #[tokio::test]
    async fn a_provider_error_stops_discovery_instead_of_inventing_tasks() {
        let samples: Vec<Sample> = (0..20).map(|index| sample(&index.to_string())).collect();

        let result = discover(
            &Failing,
            &NoEmbeddings,
            &TaxonomyModels::default(),
            &samples,
            || "id".into(),
        )
        .await;

        assert_eq!(result, Err(DiscoveryError::Provider("synthesis")));
    }
}
