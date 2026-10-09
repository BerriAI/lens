use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use lens_contract::{
    datasets::{Dataset, DatasetSummary},
    ingestion::IngestionKey,
    investigations::{Lens, Worker},
    signals::{SignalConfig, StoredTraceSignal},
    worker::{Job, Review},
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Error;

const KEY_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LegacySnapshot {
    pub(crate) lenses: Vec<LensRow>,
    pub(crate) runs: Vec<RunRow>,
    pub(crate) reviews: Vec<ReviewRow>,
    pub(crate) workers: Vec<WorkerRow>,
    pub(crate) ingestion_keys: Vec<Document>,
    pub(crate) datasets: Vec<DatasetRow>,
    pub(crate) signal_configs: Vec<Document>,
    pub(crate) trace_signals: Vec<StoredTraceSignal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    id: String,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LensRow {
    id: String,
    version: i64,
    data: Value,
    due_at: Option<NaiveDateTime>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRow {
    id: String,
    lens_id: String,
    #[serde(rename = "created_at")]
    _created_at: NaiveDateTime,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewRow {
    lens_id: String,
    criteria_key: String,
    execution_id: String,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerRow {
    id: String,
    token_hash: String,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DatasetRow {
    id: String,
    revision: i64,
    created_at: NaiveDateTime,
    data: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u32,
    pub fingerprint: String,
    pub records: usize,
    pub source_rows: BTreeMap<String, usize>,
}

pub struct Plan {
    records: BTreeMap<String, Value>,
    report: Report,
}

impl Plan {
    pub fn records(&self) -> &BTreeMap<String, Value> {
        &self.records
    }

    pub fn report(&self) -> &Report {
        &self.report
    }
}

fn key(namespace: &str, identity: impl Serialize) -> Result<String, Error> {
    Ok(format!(
        "{namespace}/{}",
        utf8_percent_encode(&serde_json::to_string(&identity)?, KEY_ENCODING)
    ))
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, Error> {
    if !value.is_object() {
        return Err(Error::InvalidRecord);
    }
    Ok(serde_json::from_value(value.clone())?)
}

fn insert(records: &mut BTreeMap<String, Value>, key: String, value: Value) -> Result<(), Error> {
    if records.insert(key, value).is_some() {
        return Err(Error::Duplicate);
    }
    Ok(())
}

impl LegacySnapshot {
    pub fn plan(self) -> Result<Plan, Error> {
        let source_rows = BTreeMap::from([
            ("lenses".into(), self.lenses.len()),
            ("runs".into(), self.runs.len()),
            ("reviews".into(), self.reviews.len()),
            ("workers".into(), self.workers.len()),
            ("ingestion_keys".into(), self.ingestion_keys.len()),
            ("datasets".into(), self.datasets.len()),
            ("signal_configs".into(), self.signal_configs.len()),
            ("trace_signals".into(), self.trace_signals.len()),
        ]);
        let mut records = BTreeMap::new();
        for row in self.lenses {
            let lens: Lens = decode(&row.data)?;
            if lens.id != row.id || row.version < 0 || lens.version < 0 || lens.revision < 0 {
                return Err(Error::InvalidRecord);
            }
            insert(
                &mut records,
                key("lens", (&row.id,))?,
                json!({"lens":row.data,"due_at":row.due_at.map(|date|date.and_utc())}),
            )?;
        }
        for row in self.runs {
            let job: Job = decode(&row.data)?;
            if job.id != row.id {
                return Err(Error::InvalidRecord);
            }
            insert(
                &mut records,
                key("run", (&row.lens_id, &row.id))?,
                json!({"lens_id":row.lens_id,"parent_key":key("lens",(&row.lens_id,))?,"archived_version":0,"job":row.data}),
            )?;
        }
        for row in self.reviews {
            let review: Review = decode(&row.data)?;
            if review.execution_id != row.execution_id {
                return Err(Error::InvalidRecord);
            }
            insert(
                &mut records,
                key(
                    "review",
                    (&row.lens_id, &row.criteria_key, &row.execution_id),
                )?,
                row.data,
            )?;
        }
        for row in self.workers {
            let worker: Worker = decode(&row.data)?;
            if worker.id != row.id || row.token_hash.is_empty() {
                return Err(Error::InvalidRecord);
            }
            insert(&mut records, key("worker", (&row.id,))?, row.data)?;
            insert(
                &mut records,
                key("worker-token", (&row.token_hash,))?,
                json!(row.id),
            )?;
        }
        let mut catalog = BTreeMap::new();
        for row in self.ingestion_keys {
            let credential: IngestionKey = decode(&row.data)?;
            if credential.id != row.id {
                return Err(Error::InvalidRecord);
            }
            if catalog.insert(row.id, row.data).is_some() {
                return Err(Error::Duplicate);
            }
        }
        if !catalog.is_empty() {
            insert(
                &mut records,
                "ingestion-keys/catalog".into(),
                json!(catalog.into_values().collect::<Vec<_>>()),
            )?;
        }
        let mut latest: BTreeMap<String, (i64, Value)> = BTreeMap::new();
        for row in self.datasets {
            let dataset: Dataset = decode(&row.data)?;
            if dataset.id != row.id || dataset.revision != row.revision || row.revision < 0 {
                return Err(Error::InvalidRecord);
            }
            let summary = json!({"team_id":dataset.team_id,"summary":DatasetSummary{
                id:dataset.id.clone(),name:dataset.name,agent_name:dataset.agent_name,
                revision:dataset.revision,case_count:dataset.cases.len(),updated_at:row.created_at.and_utc()
            }});
            if latest
                .get(&row.id)
                .is_none_or(|(revision, _)| row.revision > *revision)
            {
                latest.insert(row.id.clone(), (row.revision, summary));
            }
            insert(
                &mut records,
                key("dataset", (&row.id, row.revision))?,
                row.data,
            )?;
        }
        for (id, (_, summary)) in latest {
            insert(&mut records, key("dataset-latest", (&id,))?, summary)?;
        }
        for row in self.signal_configs {
            let _: SignalConfig = decode(&row.data)?;
            if row.id != "global" {
                return Err(Error::InvalidRecord);
            }
            insert(&mut records, "signals/config".into(), row.data)?;
        }
        for row in self.trace_signals {
            if row.span_count < 0 || !row.data.is_object() {
                return Err(Error::InvalidRecord);
            }
            insert(
                &mut records,
                key("trace-signal", (&row.trace_id, &row.trace_ref))?,
                serde_json::to_value(row)?,
            )?;
        }
        let mut fingerprint = serde_json::to_value(&records)?;
        fingerprint.sort_all_objects();
        let report = Report {
            schema: 1,
            fingerprint: format!("{:x}", Sha256::digest(serde_json::to_vec(&fingerprint)?)),
            records: records.len(),
            source_rows,
        };
        Ok(Plan { records, report })
    }
}
