use lens_contract::auth::{Identity, Role};
use litellm_traces::query::named::ReadAccessParams;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;

use crate::{Error, model::bounded_text, store::Store};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Deepagents,
    Hermes,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub runtime: Runtime,
    pub enabled: bool,
    pub created_at: i64,
    #[serde(skip)]
    pub workspace: String,
    #[serde(skip)]
    pub owner: String,
    #[serde(skip)]
    pub uid: u32,
    #[serde(skip)]
    pub scope: Option<ReadAccessParams>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotInput {
    pub id: Option<String>,
    pub name: String,
    pub instructions: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Conversation {
    pub id: String,
    pub actor_id: String,
    pub title: String,
    pub native_id: Option<String>,
    pub permanent: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Turn {
    pub id: String,
    pub conversation_id: String,
    pub input: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub sequence: i64,
    pub turn_id: String,
    pub kind: String,
    pub data: Value,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct Access {
    pub workspace: String,
    pub owner: String,
    pub scope: ReadAccessParams,
    pub writable: bool,
}

impl Access {
    pub fn for_identity(identity: &Identity) -> Result<Self, Error> {
        let writable = matches!(identity.user_role, Role::ProxyAdmin);
        let owner = identity
            .user_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or(Error::Forbidden)?;
        if matches!(
            identity.user_role,
            Role::ProxyAdmin | Role::ProxyAdminViewer
        ) {
            return Ok(Self {
                workspace: "global".into(),
                owner,
                scope: ReadAccessParams {
                    all_teams: true,
                    user_id: String::new(),
                    team_ids: vec![],
                },
                writable,
            });
        }
        if let Some(team) = identity
            .team_id
            .as_ref()
            .filter(|team| identity.log_team_ids.contains(team))
        {
            return Ok(Self {
                workspace: format!("team:{team}"),
                owner,
                scope: ReadAccessParams {
                    all_teams: false,
                    user_id: String::new(),
                    team_ids: vec![team.clone()],
                },
                writable,
            });
        }
        Ok(Self {
            workspace: format!("user:{owner}"),
            scope: ReadAccessParams {
                all_teams: false,
                user_id: owner.clone(),
                team_ids: vec![],
            },
            owner,
            writable,
        })
    }

    pub fn require_write(&self) -> Result<(), Error> {
        if self.writable {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

impl Store {
    pub async fn create_agent(
        &self,
        access: &Access,
        input: &BotInput,
        runtime: Runtime,
    ) -> Result<Agent, Error> {
        access.require_write()?;
        bounded_text(&input.name, 100)?;
        bounded_text(&input.instructions, 32_000)?;
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        if matches!(runtime, Runtime::Deepagents) {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM actors WHERE workspace=? AND owner=? AND runtime='deepagents'",
            )
            .bind(&access.workspace)
            .bind(&access.owner)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(id) = existing {
                tx.commit().await?;
                return self.agent(&access.workspace, &id).await;
            }
        }
        let id = input
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if uuid::Uuid::parse_str(&id).is_err() {
            return Err(Error::Invalid("Use a UUID idempotency key"));
        }
        if let Some(row) = sqlx::query("SELECT * FROM actors WHERE id=?")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?
        {
            let existing = agent(&row)?;
            if existing.workspace != access.workspace
                || existing.owner != access.owner
                || existing.name != input.name
                || existing.instructions != input.instructions
            {
                return Err(Error::Conflict(
                    "This bot ID already belongs to another request",
                ));
            }
            return Ok(existing);
        }
        sqlx::query("INSERT INTO actors(workspace,id,name,instructions,runtime,owner,scope,created_at) VALUES (?,?,?,?,?,?,?,?)")
            .bind(&access.workspace).bind(&id).bind(&input.name).bind(&input.instructions)
            .bind(match runtime { Runtime::Deepagents => "deepagents", Runtime::Hermes => "hermes" })
            .bind(&access.owner).bind(serde_json::to_string(&access.scope)?).bind(chrono::Utc::now().timestamp_millis())
            .execute(&mut *tx).await?;
        if matches!(runtime, Runtime::Hermes) {
            let now = chrono::Utc::now().timestamp_millis();
            sqlx::query("INSERT INTO conversations VALUES (?,?,?,NULL,1,?,?)")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&id)
                .bind("Bot Chat")
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.agent(&access.workspace, &id).await
    }

    pub async fn agent(&self, workspace: &str, id: &str) -> Result<Agent, Error> {
        let row = sqlx::query("SELECT * FROM actors WHERE workspace=? AND id=?")
            .bind(workspace)
            .bind(id)
            .fetch_optional(&self.0)
            .await?
            .ok_or(Error::NotFound)?;
        agent(&row)
    }

    pub async fn agent_for_runtime(&self, id: &str) -> Result<Agent, Error> {
        let row = sqlx::query("SELECT * FROM actors WHERE id=?")
            .bind(id)
            .fetch_optional(&self.0)
            .await?
            .ok_or(Error::NotFound)?;
        agent(&row)
    }

    pub async fn agents(&self, access: &Access) -> Result<Vec<Agent>, Error> {
        sqlx::query("SELECT * FROM actors WHERE workspace=? AND (runtime='hermes' OR owner=?) ORDER BY created_at,id")
            .bind(&access.workspace).bind(&access.owner).fetch_all(&self.0).await?.iter().map(agent).collect()
    }

    pub async fn set_enabled(
        &self,
        access: &Access,
        id: &str,
        enabled: bool,
    ) -> Result<Agent, Error> {
        access.require_write()?;
        let target = self.agent(&access.workspace, id).await?;
        if !matches!(target.runtime, Runtime::Hermes) {
            return Err(Error::Forbidden);
        }
        sqlx::query("UPDATE actors SET enabled=? WHERE workspace=? AND id=?")
            .bind(enabled)
            .bind(&access.workspace)
            .bind(id)
            .execute(&self.0)
            .await?;
        self.agent(&access.workspace, id).await
    }

    pub async fn create_conversation(
        &self,
        access: &Access,
        actor_id: &str,
        title: &str,
    ) -> Result<Conversation, Error> {
        self.create_conversation_with_id(access, actor_id, title, &uuid::Uuid::new_v4().to_string())
            .await
    }

    pub async fn create_conversation_with_id(
        &self,
        access: &Access,
        actor_id: &str,
        title: &str,
        id: &str,
    ) -> Result<Conversation, Error> {
        access.require_write()?;
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(Error::Invalid("Use a UUID idempotency key"));
        }
        bounded_text(title, 300)?;
        let actor = self.agent(&access.workspace, actor_id).await?;
        if matches!(actor.runtime, Runtime::Deepagents) && actor.owner != access.owner {
            return Err(Error::Forbidden);
        }
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO conversations VALUES (?,?,?,NULL,0,?,?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(id)
        .bind(actor_id)
        .bind(title)
        .bind(now)
        .bind(now)
        .execute(&self.0)
        .await?;
        let conversation = self.conversation(access, id).await?;
        if conversation.actor_id != actor_id
            || conversation.title != title
            || conversation.permanent
        {
            return Err(Error::Conflict(
                "This conversation ID already belongs to another request",
            ));
        }
        Ok(conversation)
    }

    pub async fn conversation(&self, access: &Access, id: &str) -> Result<Conversation, Error> {
        let row = sqlx::query("SELECT c.* FROM conversations c JOIN actors a ON a.id=c.actor_id WHERE c.id=? AND a.workspace=? AND (a.runtime='hermes' OR a.owner=?)")
            .bind(id).bind(&access.workspace).bind(&access.owner).fetch_optional(&self.0).await?.ok_or(Error::NotFound)?;
        conversation(&row)
    }

    pub async fn conversations(
        &self,
        access: &Access,
        actor_id: &str,
    ) -> Result<Vec<Conversation>, Error> {
        let actor = self.agent(&access.workspace, actor_id).await?;
        if matches!(actor.runtime, Runtime::Deepagents) && actor.owner != access.owner {
            return Err(Error::Forbidden);
        }
        sqlx::query("SELECT * FROM conversations WHERE actor_id=? ORDER BY permanent DESC,updated_at DESC LIMIT 100")
            .bind(actor_id).fetch_all(&self.0).await?.iter().map(conversation).collect()
    }

    pub async fn bind_native_session(&self, id: &str, native_id: &str) -> Result<(), Error> {
        sqlx::query("UPDATE conversations SET native_id=? WHERE id=? AND native_id IS NULL")
            .bind(native_id)
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    pub async fn admit(
        &self,
        access: &Access,
        conversation_id: &str,
        id: &str,
        input: &str,
    ) -> Result<(Turn, bool), Error> {
        access.require_write()?;
        bounded_text(input, 100_000)?;
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(Error::Invalid("A turn needs a UUID idempotency key"));
        }
        let conversation = self.conversation(access, conversation_id).await?;
        let actor = self
            .agent(&access.workspace, &conversation.actor_id)
            .await?;
        if !actor.enabled {
            return Err(Error::Conflict("Resume this bot before sending a message"));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "INSERT INTO turns VALUES (?,?,?,'queued',NULL,?,?) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(conversation_id)
        .bind(input)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query("SELECT * FROM turns WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let turn = turn(&row)?;
        if turn.conversation_id != conversation_id || turn.input != input {
            return Err(Error::Conflict(
                "This turn ID already belongs to another message",
            ));
        }
        sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
            .bind(now)
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok((turn, result.rows_affected() == 1))
    }

    pub async fn turns(&self, access: &Access, conversation_id: &str) -> Result<Vec<Turn>, Error> {
        self.conversation(access, conversation_id).await?;
        sqlx::query("SELECT * FROM turns WHERE conversation_id=? ORDER BY created_at,id")
            .bind(conversation_id)
            .fetch_all(&self.0)
            .await?
            .iter()
            .map(turn)
            .collect()
    }

    pub async fn event(&self, turn_id: &str, kind: &str, data: &Value) -> Result<(), Error> {
        sqlx::query("INSERT INTO events(turn_id,kind,data,created_at) VALUES (?,?,?,?)")
            .bind(turn_id)
            .bind(kind)
            .bind(serde_json::to_string(data)?)
            .bind(chrono::Utc::now().timestamp_millis())
            .execute(&self.0)
            .await?;
        Ok(())
    }

    pub async fn events(
        &self,
        access: &Access,
        conversation_id: &str,
        after: i64,
    ) -> Result<Vec<Event>, Error> {
        self.conversation(access, conversation_id).await?;
        sqlx::query("SELECT e.* FROM events e JOIN turns t ON t.id=e.turn_id WHERE t.conversation_id=? AND e.sequence>? ORDER BY e.sequence LIMIT 1000")
            .bind(conversation_id).bind(after).fetch_all(&self.0).await?.iter().map(|row| Ok(Event {
                sequence: row.try_get("sequence")?, turn_id: row.try_get("turn_id")?, kind: row.try_get("kind")?,
                data: serde_json::from_str(row.try_get("data")?)?, created_at: row.try_get("created_at")?,
            })).collect()
    }

    pub async fn history(&self, conversation_id: &str) -> Result<Value, Error> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT messages FROM native_history WHERE conversation_id=?")
                .bind(conversation_id)
                .fetch_optional(&self.0)
                .await?;
        text.map(|value| serde_json::from_str(&value))
            .transpose()
            .map(|value| value.unwrap_or(serde_json::Value::Null))
            .map_err(Into::into)
    }

    pub async fn cache_history(&self, conversation_id: &str, history: &Value) -> Result<(), Error> {
        sqlx::query("INSERT INTO native_history VALUES (?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET messages=excluded.messages,updated_at=excluded.updated_at")
            .bind(conversation_id).bind(serde_json::to_string(history)?).bind(chrono::Utc::now().timestamp_millis()).execute(&self.0).await?;
        Ok(())
    }

    pub(crate) async fn turn(&self, id: &str) -> Result<Turn, Error> {
        let row = sqlx::query("SELECT * FROM turns WHERE id=?")
            .bind(id)
            .fetch_one(&self.0)
            .await?;
        turn(&row)
    }

    pub(crate) async fn interrupt_actor(&self, id: &str) -> Result<(), Error> {
        sqlx::query("UPDATE turns SET status='interrupted',error='Stopped by user' WHERE conversation_id IN (SELECT id FROM conversations WHERE actor_id=?) AND status IN ('queued','running')")
            .bind(id).execute(&self.0).await?;
        Ok(())
    }

    pub async fn turn_status(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE turns SET status=?,error=?,updated_at=? WHERE id=? AND status IN ('queued','running')")
            .bind(status).bind(error).bind(chrono::Utc::now().timestamp_millis()).bind(id).execute(&self.0).await?;
        Ok(())
    }
}

fn agent(row: &sqlx::sqlite::SqliteRow) -> Result<Agent, Error> {
    Ok(Agent {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        instructions: row.try_get("instructions")?,
        runtime: match row.try_get::<&str, _>("runtime")? {
            "deepagents" => Runtime::Deepagents,
            _ => Runtime::Hermes,
        },
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
        workspace: row.try_get("workspace")?,
        owner: row.try_get("owner")?,
        uid: row.try_get::<u32, _>("uid")? + 100_000,
        scope: Some(serde_json::from_str(row.try_get("scope")?)?),
    })
}

fn conversation(row: &sqlx::sqlite::SqliteRow) -> Result<Conversation, Error> {
    Ok(Conversation {
        id: row.try_get("id")?,
        actor_id: row.try_get("actor_id")?,
        title: row.try_get("title")?,
        native_id: row.try_get("native_id")?,
        permanent: row.try_get("permanent")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn turn(row: &sqlx::sqlite::SqliteRow) -> Result<Turn, Error> {
    Ok(Turn {
        id: row.try_get("id")?,
        conversation_id: row.try_get("conversation_id")?,
        input: row.try_get("input")?,
        status: row.try_get("status")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}
