use std::collections::{BTreeMap, BTreeSet};

use futures_util::{StreamExt, TryStreamExt, stream};
use lens_contract::eval::{CreateEvalRun, FINDING_ID_META_KEY, ResolvedDataset};
use litellm_storage_clickhouse::{evals::StoredCase, state::ClickHouseState};
use litellm_traces_clickhouse::evals::EvalTraces;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use crate::{ApiError, EvalApiError};

const KEY_CHARACTERS: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn record_key(namespace: &str, identity: impl Serialize) -> Result<String, serde_json::Error> {
    let identity = serde_json::to_string(&identity)?;
    Ok(format!(
        "{namespace}/{}",
        utf8_percent_encode(&identity, KEY_CHARACTERS)
    ))
}

#[derive(Clone, Deserialize, Serialize)]
struct ToolCall {
    name: String,
    arguments: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Message {
    role: String,
    content: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct CaseSource {
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    trace_ref: String,
    #[serde(default)]
    span_id: String,
    #[serde(default)]
    finding_id: String,
    #[serde(default)]
    lens_id: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct DatasetCase {
    id: String,
    messages: Vec<Message>,
    #[serde(default)]
    reply: String,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    #[serde(default)]
    expected: String,
    #[serde(default = "included")]
    included: bool,
    #[serde(default)]
    source: CaseSource,
    #[serde(default)]
    agent_version: String,
    #[serde(default)]
    meta: BTreeMap<String, String>,
}

fn included() -> bool {
    true
}

#[derive(Deserialize)]
struct Dataset {
    id: String,
    name: String,
    team_id: String,
    revision: u64,
    cases: Vec<DatasetCase>,
}

#[derive(Serialize)]
pub(crate) struct EvalCases {
    dataset_id: String,
    revision: u64,
    cases: Vec<DatasetCase>,
}

#[derive(Deserialize)]
struct Latest {
    team_id: String,
    summary: DatasetSummary,
}

#[derive(Deserialize)]
struct DatasetSummary {
    id: String,
    name: String,
    revision: u64,
}

#[derive(Deserialize)]
struct StoredLens {
    lens: Lens,
}

#[derive(Deserialize)]
struct Lens {
    scope: Scope,
    findings: Vec<Finding>,
}

#[derive(Deserialize)]
struct Scope {
    team_id: String,
}

#[derive(Deserialize)]
struct Finding {
    id: String,
    title: String,
    priority: String,
}

pub(crate) struct Datasets {
    state: ClickHouseState,
    traces: Option<EvalTraces>,
}

impl Datasets {
    pub(crate) fn new(state: ClickHouseState, traces: Option<EvalTraces>) -> Self {
        Self { state, traces }
    }

    async fn get(&self, team: &str, id: &str, revision: u64) -> Result<Dataset, EvalApiError> {
        let latest = self
            .state
            .read(&record_key("dataset-latest", [id])?)
            .await?;
        if latest.value.is_null() {
            return Err(EvalApiError::DatasetNotFound);
        }
        let latest: Latest = serde_json::from_value(latest.value)?;
        if latest.team_id != team {
            return Err(EvalApiError::DatasetNotFound);
        }
        let snapshot = self
            .state
            .read(&record_key("dataset", (id, revision))?)
            .await?;
        if snapshot.value.is_null() {
            return Err(EvalApiError::RevisionNotFound);
        }
        let dataset: Dataset = serde_json::from_value(snapshot.value)?;
        if dataset.team_id != team || dataset.id != id {
            return Err(EvalApiError::DatasetNotFound);
        }
        if dataset.revision != revision {
            return Err(EvalApiError::RevisionNotFound);
        }
        Ok(dataset)
    }

    pub(crate) async fn resolve(
        &self,
        team: &str,
        name: &str,
        revision: Option<u64>,
    ) -> Result<ResolvedDataset, EvalApiError> {
        let mut after = String::new();
        loop {
            let keys = self.state.keys("dataset-latest/", &after, 100).await?;
            let Some(last) = keys.last() else {
                return Err(EvalApiError::DatasetNotFound);
            };
            let references: Vec<_> = keys.iter().map(String::as_str).collect();
            for snapshot in self.state.read_many(&references).await? {
                if snapshot.value.is_null() {
                    continue;
                }
                let latest: Latest = serde_json::from_value(snapshot.value)?;
                if latest.team_id == team && latest.summary.name == name {
                    let dataset = self
                        .get(
                            team,
                            &latest.summary.id,
                            revision.unwrap_or(latest.summary.revision),
                        )
                        .await?;
                    return Ok(ResolvedDataset {
                        id: dataset.id,
                        name: dataset.name,
                        revision: dataset.revision,
                    });
                }
            }
            after = last.clone();
        }
    }

    pub(crate) async fn cases(
        &self,
        team: &str,
        id: &str,
        revision: u64,
    ) -> Result<EvalCases, EvalApiError> {
        let dataset = self.get(team, id, revision).await?;
        Ok(EvalCases {
            dataset_id: dataset.id,
            revision: dataset.revision,
            cases: self
                .metadata(
                    team,
                    dataset
                        .cases
                        .into_iter()
                        .filter(|case| case.included)
                        .collect(),
                )
                .await?,
        })
    }

    async fn metadata(
        &self,
        team: &str,
        cases: Vec<DatasetCase>,
    ) -> Result<Vec<DatasetCase>, EvalApiError> {
        let attributes: BTreeMap<String, BTreeMap<String, String>> = match &self.traces {
            Some(traces) => {
                let ids: BTreeSet<_> = cases
                    .iter()
                    .map(|case| case.source.trace_id.as_str())
                    .filter(|id| !id.is_empty())
                    .collect();
                let lookups: Vec<_> = ids
                    .into_iter()
                    .map(|id| async move {
                        let attributes = traces
                            .root_attributes(team, id)
                            .await
                            .map_err(|error| ApiError::Internal(Box::new(error)))?;
                        Ok::<_, EvalApiError>((id.to_owned(), attributes))
                    })
                    .collect();
                stream::iter(lookups)
                    .buffer_unordered(8)
                    .try_collect()
                    .await?
            }
            None => BTreeMap::new(),
        };
        Ok(cases
            .into_iter()
            .map(|mut case| {
                case.meta.retain(|key, _| key == "repo_url");
                if !case.source.finding_id.is_empty() {
                    case.meta
                        .insert(FINDING_ID_META_KEY.into(), case.source.finding_id.clone());
                }
                if let Some(repo_url) = attributes
                    .get(&case.source.trace_id)
                    .and_then(|root| root.get("repo_url"))
                    .filter(|value| !value.is_empty())
                {
                    case.meta.insert("repo_url".into(), repo_url.clone());
                }
                case
            })
            .collect())
    }

    pub(crate) async fn selected_cases(
        &self,
        team: &str,
        request: &CreateEvalRun,
    ) -> Result<Vec<StoredCase>, EvalApiError> {
        let dataset = self
            .get(team, &request.dataset_id, request.revision)
            .await?;
        let selected = select_cases(&dataset.cases, request.case_ids.as_deref())?;
        let lens_ids: BTreeSet<_> = selected
            .iter()
            .map(|case| case.source.lens_id.as_str())
            .filter(|id| !id.is_empty())
            .collect();
        let mut findings = BTreeMap::new();
        for lens_id in lens_ids {
            let snapshot = self.state.read(&record_key("lens", [lens_id])?).await?;
            if snapshot.value.is_null() {
                continue;
            }
            let stored: StoredLens = serde_json::from_value(snapshot.value)?;
            if stored.lens.scope.team_id != team {
                continue;
            }
            findings.extend(
                stored
                    .lens
                    .findings
                    .into_iter()
                    .map(|finding| ((lens_id.to_owned(), finding.id.clone()), finding)),
            );
        }
        Ok(selected
            .into_iter()
            .map(|case| {
                let finding =
                    findings.get(&(case.source.lens_id.clone(), case.source.finding_id.clone()));
                StoredCase {
                    id: case.id.clone(),
                    title: finding
                        .map(|finding| finding.title.clone())
                        .unwrap_or_else(|| case.id.clone()),
                    critical: finding.is_some_and(|finding| finding.priority == "high"),
                    expected: case.expected.clone(),
                }
            })
            .collect())
    }
}

fn select_cases<'a>(
    cases: &'a [DatasetCase],
    ids: Option<&[String]>,
) -> Result<Vec<&'a DatasetCase>, EvalApiError> {
    let included: Vec<_> = cases.iter().filter(|case| case.included).collect();
    let Some(ids) = ids else {
        return Ok(included);
    };
    let requested: BTreeSet<_> = ids.iter().collect();
    if requested.len() != ids.len()
        || requested
            .iter()
            .any(|id| !included.iter().any(|case| &case.id == *id))
    {
        return Err(EvalApiError::UnknownCase);
    }
    Ok(included
        .into_iter()
        .filter(|case| requested.contains(&case.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use litellm_http::Client;
    use litellm_storage_clickhouse::{Connection, state::ClickHouseState};
    use litellm_traces_clickhouse::evals::EvalTraces;
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, query_param},
    };

    use super::{DatasetCase, Datasets, select_cases};

    #[rstest]
    #[case::source_metadata(Some("https://source.example/repo"), "https://source.example/repo")]
    #[case::missing_source(None, "https://saved.example/repo")]
    #[case::empty_source_value(Some(""), "https://saved.example/repo")]
    #[tokio::test]
    async fn should_copy_only_allowed_root_metadata_with_saved_fallback(
        #[case] source_repo: Option<&str>,
        #[case] expected: &str,
    ) {
        let server = MockServer::start().await;
        let rows: Vec<_> = source_repo
            .map(|repo| json!({"attributes":{"repo_url":repo,"secret":"private"}}))
            .into_iter()
            .collect();
        let owners: Vec<_> = source_repo
            .map(|_| json!({"trace_id":"source-trace","api_key_hash":"source-owner"}))
            .into_iter()
            .collect();
        Mock::given(method("POST"))
            .and(query_param("param_team", "team-a"))
            .and(query_param("param_attribute", "trace_id"))
            .and(query_param("param_value", "source-trace"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":owners})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(query_param("param_team", "team-a"))
            .and(query_param("param_trace_id", "source-trace"))
            .and(query_param("param_api_key_hash", "source-owner"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":rows})))
            .expect(u64::from(source_repo.is_some()))
            .mount(&server)
            .await;
        let client = Client::no_redirect_for_test();
        let connection = Connection::reader(&server.uri(), "test").unwrap();
        let datasets = Datasets::new(
            ClickHouseState::new(client.clone(), connection.clone()),
            Some(EvalTraces::new(client, connection)),
        );
        let case: DatasetCase = serde_json::from_value(json!({
            "id":"first", "messages":[], "source":{"trace_id":"source-trace"},
            "meta":{"repo_url":"https://saved.example/repo", "secret":"private"}
        }))
        .unwrap();
        let second = DatasetCase {
            id: "second".into(),
            ..case.clone()
        };
        let cases = datasets
            .metadata("team-a", vec![case, second])
            .await
            .unwrap();
        assert_eq!(
            cases[0].meta,
            std::collections::BTreeMap::from([("repo_url".into(), expected.into())])
        );
        assert_eq!(cases[1].meta, cases[0].meta);
    }

    #[rstest]
    #[tokio::test]
    async fn should_keep_allowed_metadata_without_a_trace_reader() {
        let datasets = Datasets::new(
            ClickHouseState::new(
                Client::no_redirect_for_test(),
                Connection::reader("http://127.0.0.1:9", "unused").unwrap(),
            ),
            None,
        );
        let case: DatasetCase = serde_json::from_value(json!({
            "id":"first", "messages":[], "meta":{"repo_url":"https://saved.example/repo", "secret":"private"}
        })).unwrap();
        let cases = datasets.metadata("team-a", vec![case]).await.unwrap();
        assert_eq!(
            cases[0].meta,
            std::collections::BTreeMap::from([(
                "repo_url".into(),
                "https://saved.example/repo".into()
            )])
        );
    }

    #[rstest]
    #[case::all(None)]
    #[case::subset(Some(vec!["a".to_owned(), "z".to_owned()]))]
    fn should_preserve_dataset_case_order(#[case] ids: Option<Vec<String>>) {
        let cases: Vec<DatasetCase> = serde_json::from_value(json!([
            {"id":"z","messages":[]}, {"id":"a","messages":[]}, {"id":"excluded","messages":[],"included":false}
        ])).unwrap();
        let selected = select_cases(&cases, ids.as_deref()).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|case| case.id.as_str())
                .collect::<Vec<_>>(),
            vec!["z", "a"]
        );
    }
}
