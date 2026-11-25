use std::{path::Path, time::Duration};

use serde_json::Value;
use sqlx::{
    QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow},
};

use crate::{Error, model::*};

#[derive(Clone)]
pub struct Store(pub(crate) SqlitePool);

impl Store {
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self(pool))
    }

    pub async fn create_definition(
        &self,
        actor: &Actor,
        input: DefinitionInput,
    ) -> Result<Definition, Error> {
        input.validate()?;
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        let existing = sqlx::query("SELECT d.* FROM definitions d JOIN definition_names n ON n.workspace=d.workspace AND n.definition_id=d.id WHERE n.workspace=? AND n.name=?")
            .bind(&actor.workspace).bind(&input.key).fetch_optional(&mut *tx).await?;
        if let Some(row) = existing {
            let stored: DefinitionInput = serde_json::from_str(row.try_get("definition")?)?;
            let id: String = row.try_get("id")?;
            if stored != input {
                return Err(Error::Conflict(
                    "This annotation key already has a definition. Read and reuse it or choose a different key",
                ));
            }
            tx.commit().await?;
            return self.definition(&actor.workspace, &id).await;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp_millis();
        sqlx::query("INSERT INTO definitions VALUES (?, ?, ?, ?, ?)")
            .bind(&actor.workspace)
            .bind(&id)
            .bind(serde_json::to_string(&input)?)
            .bind(&actor.producer_id)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO definition_names VALUES (?, ?, ?)")
            .bind(&actor.workspace)
            .bind(&input.key)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Definition {
            id,
            definition: input,
            aliases: vec![],
            created_by: actor.producer_id.clone(),
            created_at,
        })
    }

    pub async fn definition(&self, workspace: &str, id_or_key: &str) -> Result<Definition, Error> {
        let row = sqlx::query("SELECT d.* FROM definitions d WHERE d.workspace=? AND (d.id=? OR d.id=(SELECT definition_id FROM definition_names WHERE workspace=? AND name=?))")
            .bind(workspace).bind(id_or_key).bind(workspace).bind(id_or_key)
            .fetch_optional(&self.0).await?.ok_or(Error::NotFound)?;
        let id: String = row.try_get("id")?;
        let definition: DefinitionInput = serde_json::from_str(row.try_get("definition")?)?;
        let aliases = sqlx::query_scalar("SELECT name FROM definition_names WHERE workspace=? AND definition_id=? AND name!=? ORDER BY name")
            .bind(workspace).bind(&id).bind(&definition.key).fetch_all(&self.0).await?;
        Ok(Definition {
            id,
            definition,
            aliases,
            created_by: row.try_get("created_by")?,
            created_at: row.try_get("created_at")?,
        })
    }

    pub async fn definitions(
        &self,
        workspace: &str,
        search: &str,
        after: &str,
    ) -> Result<Vec<Definition>, Error> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT DISTINCT d.id FROM definitions d JOIN definition_names n ON n.workspace=d.workspace AND n.definition_id=d.id WHERE d.workspace=? AND d.id>? AND (instr(lower(d.definition),lower(?))>0 OR instr(n.name,lower(?))>0) ORDER BY d.id LIMIT 100")
            .bind(workspace).bind(after).bind(search).bind(search).fetch_all(&self.0).await?;
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            result.push(self.definition(workspace, &id).await?);
        }
        Ok(result)
    }

    pub async fn alias(
        &self,
        workspace: &str,
        alias: &str,
        definition_id: &str,
    ) -> Result<Definition, Error> {
        let target = self.definition(workspace, definition_id).await?;
        DefinitionInput {
            key: alias.into(),
            ..target.definition.clone()
        }
        .validate()?;
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        let old: Option<String> = sqlx::query_scalar(
            "SELECT definition_id FROM definition_names WHERE workspace=? AND name=?",
        )
        .bind(workspace)
        .bind(alias)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(old) = old.filter(|old| old != &target.id) {
            let raw: String =
                sqlx::query_scalar("SELECT definition FROM definitions WHERE workspace=? AND id=?")
                    .bind(workspace)
                    .bind(&old)
                    .fetch_one(&mut *tx)
                    .await?;
            let source: DefinitionInput = serde_json::from_str(&raw)?;
            if source.kind != target.definition.kind
                || source.allowed_values != target.definition.allowed_values
            {
                return Err(Error::Conflict(
                    "Aliases must have the same value type and allowed values",
                ));
            }
            let conflicts: i64 = sqlx::query_scalar("SELECT count(*) FROM annotations a JOIN annotations b ON a.workspace=b.workspace AND a.trace_id=b.trace_id AND a.trace_ref=b.trace_ref AND a.producer_id=b.producer_id WHERE a.workspace=? AND a.definition_id=? AND b.definition_id=? AND a.value!=b.value")
                .bind(workspace).bind(&old).bind(&target.id).fetch_one(&mut *tx).await?;
            if conflicts != 0 {
                return Err(Error::Conflict(
                    "These definitions contain conflicting judgments from the same producer. Resolve those annotations before consolidating",
                ));
            }
            sqlx::query("UPDATE OR IGNORE annotations SET definition_id=? WHERE workspace=? AND definition_id=?")
                .bind(&target.id).bind(workspace).bind(&old).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM annotations WHERE workspace=? AND definition_id=?")
                .bind(workspace)
                .bind(&old)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE definition_names SET definition_id=? WHERE workspace=? AND definition_id=?",
            )
            .bind(&target.id)
            .bind(workspace)
            .bind(&old)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO definition_names VALUES (?, ?, ?) ON CONFLICT DO NOTHING")
                .bind(workspace)
                .bind(&old)
                .bind(&target.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM definitions WHERE workspace=? AND id=?")
                .bind(workspace)
                .bind(old)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO definition_names VALUES (?, ?, ?) ON CONFLICT DO NOTHING")
            .bind(workspace)
            .bind(alias)
            .bind(&target.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.definition(workspace, &target.id).await
    }

    pub async fn annotate(&self, actor: &Actor, input: &[AnnotationInput]) -> Result<usize, Error> {
        if input.is_empty() || input.len() > 1000 {
            return Err(Error::Invalid(
                "Write between 1 and 1000 annotations per batch",
            ));
        }
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        for annotation in input {
            bounded_text(&annotation.trace.trace_id, 256)?;
            bounded_text(&annotation.trace.trace_ref, 2048)?;
            bounded_text(&annotation.evidence, 16_384)?;
            let row = sqlx::query("SELECT d.id,d.definition FROM definitions d WHERE d.workspace=? AND (d.id=? OR d.id=(SELECT definition_id FROM definition_names WHERE workspace=? AND name=?))")
                .bind(&actor.workspace).bind(&annotation.definition_id).bind(&actor.workspace).bind(&annotation.definition_id)
                .fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
            let definition: DefinitionInput = serde_json::from_str(row.try_get("definition")?)?;
            if !definition.accepts(&annotation.value) {
                return Err(Error::Invalid(
                    "Annotation value does not match its definition",
                ));
            }
            sqlx::query("INSERT INTO annotations (workspace,trace_id,trace_ref,producer_id,definition_id,value,evidence,execution_id,updated_at) VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(workspace,trace_id,trace_ref,producer_id,definition_id) DO UPDATE SET value=excluded.value,evidence=excluded.evidence,execution_id=excluded.execution_id,updated_at=excluded.updated_at")
                .bind(&actor.workspace).bind(&annotation.trace.trace_id).bind(&annotation.trace.trace_ref)
                .bind(&actor.producer_id).bind(row.try_get::<String,_>("id")?).bind(serde_json::to_string(&annotation.value)?)
                .bind(&annotation.evidence).bind(&actor.execution_id).bind(chrono::Utc::now().timestamp_millis())
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(input.len())
    }

    pub async fn annotation_activity(
        &self,
        workspace: &str,
    ) -> Result<Vec<AnnotationActivity>, Error> {
        sqlx::query("SELECT a.producer_id,COALESCE(p.name,'Lens') AS producer_name,a.definition_id,json_extract(d.definition,'$.key') AS definition_key,a.value,count(*) AS count,max(a.updated_at) AS updated_at FROM annotations a JOIN definitions d ON d.workspace=a.workspace AND d.id=a.definition_id LEFT JOIN actors p ON p.id=a.producer_id AND p.workspace=a.workspace WHERE a.workspace=? GROUP BY a.producer_id,a.definition_id,a.value ORDER BY updated_at DESC,a.producer_id,a.definition_id,a.value LIMIT 100")
            .bind(workspace).fetch_all(&self.0).await?.iter().map(|row| Ok(AnnotationActivity {
                producer_id: row.try_get("producer_id")?, producer_name: row.try_get("producer_name")?,
                definition_id: row.try_get("definition_id")?, definition_key: row.try_get("definition_key")?,
                value: serde_json::from_str(row.try_get("value")?)?, count: row.try_get("count")?, updated_at: row.try_get("updated_at")?,
            })).collect()
    }

    pub async fn annotations(
        &self,
        workspace: &str,
        filter: &AnnotationFilter,
    ) -> Result<AnnotationPage, Error> {
        let limit = filter.limit.unwrap_or(100).clamp(1, 1000);
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT a.*, json_extract(d.definition,'$.key') AS definition_key, COALESCE(p.name, a.producer_id) AS producer_name FROM annotations a JOIN definitions d ON d.workspace=a.workspace AND d.id=a.definition_id LEFT JOIN actors p ON p.id=a.producer_id AND p.workspace=a.workspace WHERE a.workspace=",
        );
        query
            .push_bind(workspace)
            .push(" AND a.sequence>")
            .push_bind(filter.after.unwrap_or(0));
        if let Some(id) = &filter.definition_id {
            let definition = self.definition(workspace, id).await?;
            query.push(" AND a.definition_id=").push_bind(definition.id);
        }
        if let Some(id) = &filter.producer_id {
            query.push(" AND producer_id=").push_bind(id);
        }
        if let Some(value) = &filter.value {
            query
                .push(" AND value=")
                .push_bind(serde_json::to_string(value)?);
        }
        if let Some(id) = &filter.trace_id {
            query.push(" AND trace_id=").push_bind(id);
        }
        if let Some(reference) = &filter.trace_ref {
            query.push(" AND trace_ref=").push_bind(reference);
        }
        query
            .push(" ORDER BY a.sequence LIMIT ")
            .push_bind(limit + 1);
        let rows = query.build().fetch_all(&self.0).await?;
        let next_cursor = if rows.len() > limit as usize {
            Some(rows[limit as usize - 1].try_get("sequence")?)
        } else {
            None
        };
        let annotations = rows
            .iter()
            .take(limit as usize)
            .map(annotation)
            .collect::<Result<_, _>>()?;
        Ok(AnnotationPage {
            annotations,
            next_cursor,
        })
    }

    pub async fn save_report(&self, actor: &Actor, input: &ReportInput) -> Result<Report, Error> {
        bounded_text(&input.id, 128)?;
        bounded_text(&input.title, 300)?;
        bounded_text(&input.markdown, 2_000_000)?;
        if input.traces.len() > 1000 {
            return Err(Error::Invalid("Link at most 1000 traces per report"));
        }
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        let owner: Option<String> =
            sqlx::query_scalar("SELECT producer_id FROM reports WHERE workspace=? AND id=?")
                .bind(&actor.workspace)
                .bind(&input.id)
                .fetch_optional(&mut *tx)
                .await?;
        if owner.is_some_and(|owner| owner != actor.producer_id) {
            return Err(Error::Conflict(
                "This report belongs to another producer. Save your report with a new ID",
            ));
        }
        let updated_at = chrono::Utc::now().timestamp_millis();
        sqlx::query("INSERT INTO reports VALUES (?,?,?,?,?,?,?) ON CONFLICT(workspace,id) DO UPDATE SET execution_id=excluded.execution_id,title=excluded.title,markdown=excluded.markdown,updated_at=excluded.updated_at")
            .bind(&actor.workspace).bind(&input.id).bind(&actor.producer_id).bind(&actor.execution_id)
            .bind(&input.title).bind(&input.markdown).bind(updated_at).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM report_traces WHERE workspace=? AND report_id=?")
            .bind(&actor.workspace)
            .bind(&input.id)
            .execute(&mut *tx)
            .await?;
        for trace in &input.traces {
            sqlx::query("INSERT INTO report_traces VALUES (?,?,?,?) ON CONFLICT DO NOTHING")
                .bind(&actor.workspace)
                .bind(&input.id)
                .bind(&trace.trace_id)
                .bind(&trace.trace_ref)
                .execute(&mut *tx)
                .await?;
        }
        if input.artifact_ids.len() > 20 {
            return Err(Error::Invalid("Attach at most 20 artifacts to one report"));
        }
        sqlx::query("DELETE FROM report_artifacts WHERE workspace=? AND report_id=?")
            .bind(&actor.workspace)
            .bind(&input.id)
            .execute(&mut *tx)
            .await?;
        for (position, id) in input.artifact_ids.iter().enumerate() {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM artifacts WHERE workspace=? AND id=?)",
            )
            .bind(&actor.workspace)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(Error::NotFound);
            }
            sqlx::query("INSERT INTO report_artifacts VALUES (?,?,?,?) ON CONFLICT DO NOTHING")
                .bind(&actor.workspace)
                .bind(&input.id)
                .bind(id)
                .bind(position as i64)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(Report {
            preview: false,
            artifacts: self.report_artifacts(&actor.workspace, &input.id).await?,
            report: input.clone(),
            producer_id: actor.producer_id.clone(),
            execution_id: actor.execution_id.clone(),
            updated_at,
        })
    }

    pub async fn report(&self, workspace: &str, id: &str) -> Result<Report, Error> {
        self.report_text(workspace, id, false).await
    }

    async fn report_text(&self, workspace: &str, id: &str, preview: bool) -> Result<Report, Error> {
        let sql = if preview {
            "SELECT id,title,producer_id,execution_id,updated_at,substr(markdown,1,5000) AS markdown,length(markdown)>5000 AS preview FROM reports WHERE workspace=? AND id=?"
        } else {
            "SELECT *,0 AS preview FROM reports WHERE workspace=? AND id=?"
        };
        let row = sqlx::query(sql)
            .bind(workspace)
            .bind(id)
            .fetch_optional(&self.0)
            .await?
            .ok_or(Error::NotFound)?;
        let traces = sqlx::query(
            "SELECT trace_id,trace_ref FROM report_traces WHERE workspace=? AND report_id=?",
        )
        .bind(workspace)
        .bind(id)
        .fetch_all(&self.0)
        .await?
        .iter()
        .map(|row| {
            Ok(TraceIdentity {
                trace_id: row.try_get("trace_id")?,
                trace_ref: row.try_get("trace_ref")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()?;
        let artifacts = self.report_artifacts(workspace, id).await?;
        Ok(Report {
            preview: row.try_get("preview")?,
            report: ReportInput {
                artifact_ids: artifacts.iter().map(|a| a.id.clone()).collect(),
                id: id.into(),
                title: row.try_get("title")?,
                markdown: row.try_get("markdown")?,
                traces,
            },
            artifacts,
            producer_id: row.try_get("producer_id")?,
            execution_id: row.try_get("execution_id")?,
            updated_at: row.try_get("updated_at")?,
        })
    }

    pub async fn reports(
        &self,
        workspace: &str,
        trace: Option<&TraceIdentity>,
        before: Option<&str>,
    ) -> Result<Vec<Report>, Error> {
        let mut query = QueryBuilder::<Sqlite>::new("SELECT id FROM reports WHERE workspace=");
        query.push_bind(workspace);
        if let Some(cursor) = before {
            let (timestamp, id) = cursor
                .split_once(':')
                .ok_or(Error::Invalid("Invalid report cursor"))?;
            let timestamp = timestamp
                .parse::<i64>()
                .map_err(|_| Error::Invalid("Invalid report cursor"))?;
            query
                .push(" AND (updated_at<")
                .push_bind(timestamp)
                .push(" OR (updated_at=")
                .push_bind(timestamp)
                .push(" AND id<")
                .push_bind(id)
                .push("))");
        }
        if let Some(trace) = trace {
            query
                .push(" AND id IN (SELECT report_id FROM report_traces WHERE workspace=")
                .push_bind(workspace)
                .push(" AND trace_id=")
                .push_bind(&trace.trace_id)
                .push(" AND trace_ref=")
                .push_bind(&trace.trace_ref)
                .push(")");
        }
        query.push(" ORDER BY updated_at DESC,id DESC LIMIT 50");
        let ids: Vec<String> = query.build_query_scalar().fetch_all(&self.0).await?;
        let mut reports = Vec::with_capacity(ids.len());
        for id in ids {
            reports.push(self.report_text(workspace, &id, true).await?);
        }
        Ok(reports)
    }
}

fn annotation(row: &SqliteRow) -> Result<Annotation, Error> {
    Ok(Annotation {
        definition_key: row.try_get("definition_key")?,
        producer_name: row.try_get("producer_name")?,
        annotation: AnnotationInput {
            trace: TraceIdentity {
                trace_id: row.try_get("trace_id")?,
                trace_ref: row.try_get("trace_ref")?,
            },
            definition_id: row.try_get("definition_id")?,
            value: serde_json::from_str::<Value>(row.try_get("value")?)?,
            evidence: row.try_get("evidence")?,
        },
        producer_id: row.try_get("producer_id")?,
        execution_id: row.try_get("execution_id")?,
        updated_at: row.try_get("updated_at")?,
    })
}
