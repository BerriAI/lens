use lens_contract::datasets::{
    BuildRequest, BuildSource, CaseSource, DatasetCase, DatasetMessage, DatasetRole, TextSource,
    TraceSource,
};
use lens_datasets::{DatasetReader, Finding, ReadError, Scope};
use litellm_traces::{
    ObservationType, Span, SpanDetail, SpanStatus, Trace, TraceSummary, to_ui_content,
};
use std::{collections::BTreeMap, sync::Mutex};

pub fn message(content: &str) -> DatasetMessage {
    DatasetMessage {
        role: DatasetRole::User,
        content: content.into(),
        name: String::new(),
        tool_calls: Vec::new(),
    }
}

pub fn case(content: &str) -> DatasetCase {
    DatasetCase {
        id: String::new(),
        messages: vec![message(content)],
        reply: String::new(),
        tool_calls: Vec::new(),
        expected: String::new(),
        included: true,
        source: CaseSource::default(),
        agent_version: String::new(),
    }
}

pub fn text(text: &str) -> BuildSource {
    BuildSource::Text(TextSource { text: text.into() })
}

pub fn trace_source(span_id: &str) -> BuildSource {
    BuildSource::Trace(TraceSource {
        trace_id: "t1".into(),
        trace_ref: "ref".into(),
        span_id: span_id.into(),
    })
}

pub fn request(sources: Vec<BuildSource>) -> BuildRequest {
    BuildRequest {
        sources,
        dataset_id: String::new(),
    }
}

pub fn span(span_id: &str, kind: ObservationType, offset: f64) -> Span {
    Span {
        span_id: span_id.into(),
        parent_span_id: None,
        name: span_id.into(),
        kind,
        agent: "support".into(),
        framework: String::new(),
        start_offset_ms: offset,
        duration_ms: 1.0,
        status: SpanStatus::Ok,
        error: None,
        error_truncated: false,
        input_preview: String::new(),
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        litellm_request_id: None,
        spend: None,
        spend_log_request_id: None,
        spend_match: None,
    }
}

pub fn trace(spans: Vec<Span>) -> Trace {
    Trace {
        summary: TraceSummary {
            resolution_limited: false,
            trace_id: "t1".into(),
            trace_ref: "ref".into(),
            name: "run".into(),
            service: "svc".into(),
            agent_names: vec![],
            frameworks: vec![],
            input_preview: String::new(),
            start_time: String::new(),
            duration_ms: 1.0,
            status: SpanStatus::Ok,
            span_count: spans.len() as u64,
            agent_count: 1,
            agent_invocations: 1,
            llm_calls: 1,
            tool_calls: 0,
            error_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            models: vec![],
            spend: None,
            priced_calls: 0,
            source: None,
        },
        agents: vec![],
        spans,
        next_cursor: None,
        gateway_spend_pending: false,
    }
}

#[derive(Clone)]
pub struct Detail {
    pub input: String,
    pub output: String,
    pub raw_input: String,
    pub raw_output: String,
    pub attributes: BTreeMap<String, String>,
}

impl Detail {
    pub fn chat(question: &str) -> Self {
        Self { input: serde_json::json!([{"role":"system","content":"Be terse"},{"role":"user","content":question}]).to_string(), output: serde_json::json!([{"role":"assistant","content":"Done","tool_calls":[{"name":"search","arguments":"{}"}]}]).to_string(), raw_input: question.into(), raw_output: "Done".into(), attributes: BTreeMap::new() }
    }

    pub fn raw(input: &str, output: &str) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            raw_input: input.into(),
            raw_output: output.into(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn span(&self, span_id: &str) -> SpanDetail {
        SpanDetail {
            span_id: span_id.into(),
            input_ui: to_ui_content(&self.input),
            output_ui: to_ui_content(&self.output),
            input: self.raw_input.clone(),
            output: self.raw_output.clone(),
            attributes: self.attributes.clone(),
        }
    }
}

#[derive(Default)]
pub struct Reader {
    pub spans: BTreeMap<(String, String), Detail>,
    pub trace: Option<Trace>,
    pub stored: Vec<Finding>,
    pub reads: Mutex<Vec<String>>,
    pub failure: bool,
}

impl Reader {
    pub fn with_spans(spans: &[(&str, Detail)]) -> Self {
        Self {
            spans: spans
                .iter()
                .map(|(id, detail)| (("t1".into(), (*id).into()), detail.clone()))
                .collect(),
            ..Self::default()
        }
    }
}

impl DatasetReader for Reader {
    async fn trace(&self, trace_id: &str, _trace_ref: &str) -> Result<Option<Trace>, ReadError> {
        self.reads.lock().unwrap().push(format!("trace:{trace_id}"));
        if self.failure {
            return Err(ReadError::LensNotFound);
        }
        Ok(if trace_id == "t1" {
            self.trace.clone()
        } else {
            None
        })
    }

    async fn span(
        &self,
        trace_id: &str,
        span_id: &str,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, ReadError> {
        self.reads
            .lock()
            .unwrap()
            .push(format!("{trace_id}:{span_id}:{trace_ref}"));
        if self.failure {
            return Err(ReadError::LensNotFound);
        }
        Ok(self
            .spans
            .get(&(trace_id.into(), span_id.into()))
            .map(|detail| detail.span(span_id)))
    }

    async fn findings(
        &self,
        lens_id: &str,
        ids: &[String],
        _scope: &Scope,
    ) -> Result<Vec<Finding>, ReadError> {
        self.reads
            .lock()
            .unwrap()
            .push(format!("finding:{lens_id}"));
        if self.failure {
            return Err(ReadError::LensNotFound);
        }
        Ok(self
            .stored
            .iter()
            .filter(|finding| ids.contains(&finding.id))
            .cloned()
            .collect())
    }
}
