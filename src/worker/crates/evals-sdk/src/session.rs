use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    client::Client,
    engine,
    model::{
        Case, CaseError, CaseResult, EvalRun, EvalSpec, Execution, Report, RunStatus, TrialResult,
    },
    named,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestCase {
    #[serde(flatten)]
    pub case: Case,
    pub trial: usize,
}

pub struct Evaluation {
    client: Client,
    spec: EvalSpec,
    run: EvalRun,
    cases: Vec<TestCase>,
    results: Vec<TrialResult>,
    report: Option<Report>,
}

impl Evaluation {
    pub async fn start(
        name: &str,
        endpoint: &str,
        key: &str,
        execution: &Execution,
    ) -> Result<Self> {
        let prepared = named::prepare(name, endpoint, key, execution).await?;
        let body = crate::model::CreateEvalRun {
            agent_io: None,
            ..prepared.body
        };
        let run = engine::create_run(&prepared.client, &prepared.spec, &prepared.execution, &body)
            .await?;
        let cases = if matches!(run.status, RunStatus::Running) {
            prepared
                .cases
                .into_iter()
                .flat_map(|case| {
                    (0..prepared.spec.trials).map(move |trial| TestCase {
                        case: case.clone(),
                        trial,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            client: prepared.client,
            spec: prepared.spec,
            run,
            cases,
            results: Vec::new(),
            report: None,
        })
    }

    pub fn cases(&self) -> &[TestCase] {
        &self.cases
    }

    pub fn run(&self) -> &EvalRun {
        &self.run
    }

    pub async fn record(&mut self, case: &TestCase, result: CaseResult) -> Result<()> {
        if self.report.is_some() || !matches!(self.run.status, RunStatus::Running) {
            return Err(Error::Configuration(
                "Cannot record results after finishing an evaluation",
            ));
        }
        let saved = self
            .cases
            .iter()
            .find(|saved| saved.case.id == case.case.id && saved.trial == case.trial)
            .ok_or(Error::Configuration(
                "Result references a case or trial outside the saved evaluation",
            ))?;
        if saved.case.input != case.case.input || saved.case.followups != case.case.followups {
            return Err(Error::Configuration(
                "Recorded input must match the saved dataset case",
            ));
        }
        if self
            .results
            .iter()
            .any(|recorded| recorded.case_id == case.case.id && recorded.trial == case.trial)
        {
            return Err(Error::Configuration(
                "This case trial already has a recorded result",
            ));
        }
        result.validate()?;
        let result = CaseResult {
            error: result.error.map(|error| CaseError {
                message: engine::redact(&error.message),
                ..error
            }),
            ..result
        };
        self.client
            .result(&self.run.id, &case.case.id, case.trial, &result)
            .await?;
        self.results.push(TrialResult {
            case_id: case.case.id.clone(),
            trial: case.trial,
            result,
        });
        Ok(())
    }

    pub async fn finish(&mut self, failure: Option<CaseError>) -> Result<Report> {
        if let Some(report) = &self.report {
            return Ok(report.clone());
        }
        let pending = self
            .cases
            .iter()
            .filter(|case| {
                !self
                    .results
                    .iter()
                    .any(|result| result.case_id == case.case.id && result.trial == case.trial)
            })
            .cloned()
            .collect::<Vec<_>>();
        for case in pending {
            let result = match &failure {
                Some(failure) => engine::failure(&failure.r#type, &failure.message),
                None => engine::failure(
                    "MissingResult",
                    "No result was recorded for this saved case trial",
                ),
            };
            self.record(&case, result).await?;
        }
        let report = engine::complete_run(
            &self.client,
            &self.spec,
            self.run.clone(),
            self.results.clone(),
        )
        .await?;
        self.run = report.run.clone();
        self.report = Some(report.clone());
        Ok(report)
    }
}
