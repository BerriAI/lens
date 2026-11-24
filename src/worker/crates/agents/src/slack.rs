use std::{collections::HashMap, sync::Arc, time::Duration};

use agentchat::{
    AgentChat, ConversationId, ConversationPermit, Message, MessageId, State,
    slack::{Slack, SlackConfig, ThreadSubscriptions},
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio::sync::Mutex;

use crate::{
    Error,
    conversations::{Access, Agent},
    model::bounded_text,
    service::Control,
    store::Store,
};

fn channel_error(_: impl std::fmt::Display) -> agentchat::Error {
    agentchat::Error::Transport("Lens could not complete the Slack operation".into())
}

pub struct SlackState {
    store: Store,
    locks: Mutex<HashMap<ConversationId, Arc<Mutex<()>>>>,
}

impl SlackState {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            locks: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl State for SlackState {
    async fn claim(&self, id: &MessageId) -> agentchat::Result<bool> {
        Ok(
            sqlx::query("INSERT INTO slack_claims VALUES (?) ON CONFLICT DO NOTHING")
                .bind(id.as_ref())
                .execute(&self.store.0)
                .await
                .map_err(channel_error)?
                .rows_affected()
                == 1,
        )
    }
    async fn acquire(&self, id: &ConversationId) -> agentchat::Result<Box<dyn ConversationPermit>> {
        let lock = self
            .locks
            .lock()
            .await
            .entry(id.clone())
            .or_default()
            .clone();
        Ok(Box::new(lock.lock_owned().await))
    }
    async fn append(&self, message: Message) -> agentchat::Result<()> {
        sqlx::query("INSERT INTO slack_messages VALUES (?,?,?) ON CONFLICT DO NOTHING")
            .bind(message.id.as_ref())
            .bind(message.conversation_id.as_ref())
            .bind(serde_json::to_string(&message).map_err(channel_error)?)
            .execute(&self.store.0)
            .await
            .map_err(channel_error)?;
        Ok(())
    }
    async fn history(
        &self,
        id: &ConversationId,
        limit: Option<usize>,
    ) -> agentchat::Result<Vec<Message>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT data FROM (SELECT rowid,data FROM slack_messages WHERE conversation_id=? ORDER BY rowid DESC LIMIT ?) ORDER BY rowid")
            .bind(id.as_ref()).bind(limit.filter(|limit|*limit>0).map_or(-1,|limit|limit as i64)).fetch_all(&self.store.0).await.map_err(channel_error)?;
        rows.iter()
            .map(|text| serde_json::from_str(text).map_err(channel_error))
            .collect()
    }
}

#[async_trait]
impl ThreadSubscriptions for SlackState {
    async fn contains(&self, id: &str) -> agentchat::Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM slack_threads WHERE slack_conversation_id=?)",
        )
        .bind(id)
        .fetch_one(&self.store.0)
        .await
        .map_err(channel_error)
    }
    async fn add(&self, id: &str) -> agentchat::Result<()> {
        if self.contains(id).await? {
            Ok(())
        } else {
            Err(channel_error("Thread is not linked"))
        }
    }
}

#[derive(Serialize)]
pub struct LinkCode {
    pub command: String,
    pub expires_at: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlackMessage {
    pub id: String,
    pub conversation_id: String,
    pub text: String,
}

impl Store {
    pub async fn slack_code(&self, access: &Access, conversation: &str) -> Result<LinkCode, Error> {
        access.require_write()?;
        self.conversation(access, conversation).await?;
        let code = uuid::Uuid::new_v4().simple().to_string();
        let digest = format!("{:x}", Sha256::digest(code.as_bytes()));
        let expires_at = chrono::Utc::now().timestamp_millis() + 600_000;
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM slack_link_codes WHERE conversation_id=? OR expires_at<?")
            .bind(conversation)
            .bind(chrono::Utc::now().timestamp_millis())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO slack_link_codes VALUES (?,?,?,?)")
            .bind(digest)
            .bind(conversation)
            .bind(&access.owner)
            .bind(expires_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(LinkCode {
            command: format!("lens link {code}"),
            expires_at,
        })
    }

    async fn link_slack(&self, code: &str, message: &Message) -> Result<(), Error> {
        let digest = format!("{:x}", Sha256::digest(code.as_bytes()));
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("DELETE FROM slack_link_codes WHERE digest=? AND expires_at>? RETURNING conversation_id,owner")
            .bind(digest).bind(chrono::Utc::now().timestamp_millis()).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let mut source = message.clone();
        source.text = "Linked this thread to Lens".into();
        sqlx::query("INSERT INTO slack_threads VALUES (?,?,?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET slack_conversation_id=excluded.slack_conversation_id,sender_id=excluded.sender_id,owner=excluded.owner,source=excluded.source")
            .bind(row.try_get::<&str,_>("conversation_id")?).bind(message.conversation_id.as_ref()).bind(&message.sender.id)
            .bind(row.try_get::<&str,_>("owner")?).bind(serde_json::to_string(&source)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn slack_access(&self, message: &Message) -> Result<Option<(Access, String)>, Error> {
        let row = sqlx::query("SELECT s.conversation_id,s.owner,a.workspace,a.scope FROM slack_threads s JOIN conversations c ON c.id=s.conversation_id JOIN actors a ON a.id=c.actor_id WHERE s.slack_conversation_id=? AND s.sender_id=?")
            .bind(message.conversation_id.as_ref()).bind(&message.sender.id).fetch_optional(&self.0).await?;
        row.map(|row| {
            Ok((
                Access {
                    workspace: row.try_get("workspace")?,
                    owner: row.try_get("owner")?,
                    scope: serde_json::from_str(row.try_get("scope")?)?,
                    writable: true,
                },
                row.try_get("conversation_id")?,
            ))
        })
        .transpose()
    }

    pub async fn unlink_slack(&self, access: &Access, conversation: &str) -> Result<(), Error> {
        access.require_write()?;
        self.conversation(access, conversation).await?;
        let mut tx = self.0.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM slack_threads WHERE conversation_id=?")
            .bind(conversation)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM slack_link_codes WHERE conversation_id=?")
            .bind(conversation)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn slack_link(&self, access: &Access, conversation: &str) -> Result<Value, Error> {
        self.conversation(access, conversation).await?;
        let source: Option<String> =
            sqlx::query_scalar("SELECT source FROM slack_threads WHERE conversation_id=?")
                .bind(conversation)
                .fetch_optional(&self.0)
                .await?;
        Ok(match source {
            Some(source) => {
                let source: Message = serde_json::from_str(&source)?;
                json!({"linked":true,"sender_id":source.sender.id,"channel_id":source.metadata["channel_id"]})
            }
            None => json!({"linked":false}),
        })
    }

    pub async fn slack_destinations(&self, actor: &Agent) -> Result<Value, Error> {
        let rows: Vec<(String,String)> = sqlx::query_as("SELECT s.conversation_id,s.source FROM slack_threads s JOIN conversations c ON c.id=s.conversation_id WHERE c.actor_id=?")
            .bind(&actor.id).fetch_all(&self.0).await?;
        let values = rows
            .iter()
            .map(|(id, source)| {
                let source: Message = serde_json::from_str(source)?;
                Ok(json!({"conversation_id":id,"channel_id":source.metadata["channel_id"]}))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(json!(values))
    }
}

pub struct Bridge {
    slack: Arc<Slack>,
    app: Arc<AgentChat>,
}

impl Bridge {
    pub fn from_env(store: Store) -> Result<Option<Arc<Self>>, Error> {
        let bot = std::env::var("LENS_AGENT_SLACK_BOT_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
        let app = std::env::var("LENS_AGENT_SLACK_APP_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
        match (bot, app) {
            (None, None) => Ok(None),
            (Some(bot), Some(app)) => {
                let state = Arc::new(SlackState::new(store));
                let mut config = SlackConfig::new(bot, app);
                config.ack_emoji = None;
                let slack = Slack::with_thread_subscriptions(config, state.clone())
                    .map_err(|_| Error::Invalid("Slack configuration is invalid"))?;
                Ok(Some(Arc::new(Self {
                    slack: Arc::new(slack),
                    app: Arc::new(AgentChat::with_state(state)),
                })))
            }
            _ => Err(Error::Invalid(
                "Set both LENS_AGENT_SLACK_BOT_TOKEN and LENS_AGENT_SLACK_APP_TOKEN",
            )),
        }
    }

    pub async fn run(self: Arc<Self>, control: Control) -> Result<(), Error> {
        self.slack
            .authenticate()
            .await
            .map_err(|_| Error::Invalid("Slack authentication failed"))?;
        let bridge = self.clone();
        self.slack
            .run(move |message| {
                let bridge = bridge.clone();
                let control = control.clone();
                async move {
                    bridge
                        .receive(control, message)
                        .await
                        .map_err(channel_error)
                }
            })
            .await
            .map_err(|_| Error::Runtime)
    }

    async fn receive(&self, control: Control, mut message: Message) -> Result<(), Error> {
        let linked = if let Some(code) = message.text.trim().strip_prefix("lens link ") {
            if code.len() != 32 || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(());
            }
            control.store().link_slack(code, &message).await?;
            message.text = "Linked this thread to Lens".into();
            true
        } else {
            false
        };
        let Some((access, conversation)) = control.store().slack_access(&message).await? else {
            return Ok(());
        };
        self.slack
            .subscribe(&message)
            .await
            .map_err(|_| Error::Runtime)?;
        self.app.receive(self.slack.clone(),message,move |context| async move {
            if linked { return Ok(Some("Connected to Lens. Messages from the person who linked this thread continue the same conversation. You can disconnect it in Lens.".into())); }
            let id = uuid::Uuid::new_v4().to_string();
            let (turn,created) = control.store().admit(&access,&conversation,&id,&context.message.text).await.map_err(channel_error)?;
            if created { control.run(&access,turn.clone()).await.map_err(channel_error)?; }
            let response = tokio::time::timeout(Duration::from_secs(1800),async {
                loop {
                    let turn = control.store().turn(&turn.id).await?;
                    if !matches!(turn.status.as_str(),"queued"|"running") {
                        return if turn.status=="completed" { control.reply_text(&access,&conversation,&turn.id).await } else { Ok("This turn stopped. Review the conversation in Lens before continuing.".into()) };
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }).await.map_err(channel_error)?.map_err(channel_error)?;
            Ok(Some(response))
        }).await.map_err(|_|Error::Invalid("Slack delivery could not be confirmed. Check the thread before sending again"))
    }

    pub async fn send(
        &self,
        store: &Store,
        actor: &Agent,
        input: SlackMessage,
    ) -> Result<Value, Error> {
        bounded_text(&input.id, 128)?;
        bounded_text(&input.text, 30_000)?;
        let source: String = sqlx::query_scalar("SELECT s.source FROM slack_threads s JOIN conversations c ON c.id=s.conversation_id WHERE c.actor_id=? AND s.conversation_id=?")
            .bind(&actor.id).bind(&input.conversation_id).fetch_optional(&store.0).await?.ok_or(Error::Forbidden)?;
        let source: Message = serde_json::from_str(&source)?;
        let admitted = sqlx::query(
            "INSERT INTO slack_deliveries VALUES (?,?,?,?,NULL) ON CONFLICT DO NOTHING",
        )
        .bind(&actor.id)
        .bind(&input.id)
        .bind(&input.conversation_id)
        .bind(&input.text)
        .execute(&store.0)
        .await?
        .rows_affected()
            == 1;
        if !admitted {
            let row = sqlx::query("SELECT conversation_id,text,receipt FROM slack_deliveries WHERE actor_id=? AND id=?").bind(&actor.id).bind(&input.id).fetch_one(&store.0).await?;
            if row.try_get::<&str, _>("conversation_id")? != input.conversation_id
                || row.try_get::<&str, _>("text")? != input.text
            {
                return Err(Error::Conflict(
                    "This delivery ID already belongs to another message",
                ));
            }
            let receipt: Option<String> = row.try_get("receipt")?;
            return Ok(
                json!({"status":if receipt.is_some(){"sent"}else{"uncertain"},"message_id":receipt}),
            );
        }
        let reply = self.app.reply(self.slack.clone(),&source,&input.text).await.map_err(|_|Error::Invalid("Slack delivery is uncertain. Do not retry with a new ID without checking the thread"))?;
        sqlx::query("UPDATE slack_deliveries SET receipt=? WHERE actor_id=? AND id=?")
            .bind(reply.id.as_ref())
            .bind(&actor.id)
            .bind(&input.id)
            .execute(&store.0)
            .await?;
        Ok(json!({"status":"sent","message_id":reply.id}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::{BotInput, Conversation, Runtime};
    use litellm_traces::query::named::ReadAccessParams;
    use rstest::{fixture, rstest};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    struct Fixture {
        _directory: tempfile::TempDir,
        store: Store,
        access: Access,
        actor: Agent,
        chat: Conversation,
        message: Message,
    }
    #[fixture]
    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("state.db"))
            .await
            .unwrap();
        let access = Access {
            workspace: "global".into(),
            owner: "admin".into(),
            scope: ReadAccessParams {
                all_teams: true,
                user_id: String::new(),
                team_ids: vec![],
            },
            writable: true,
        };
        let actor = store
            .create_agent(
                &access,
                &BotInput {
                    id: None,
                    name: "Reviewer".into(),
                    instructions: "Review requests".into(),
                },
                Runtime::Hermes,
            )
            .await
            .unwrap();
        let chat = store
            .conversations(&access, &actor.id)
            .await
            .unwrap()
            .remove(0);
        let message:Message=serde_json::from_value(json!({"id":"slack:TTEST:CTEST:1.000001","conversation_id":"slack:TTEST:CTEST:1.000001","channel":"slack","sender":{"id":"UTEST"},"role":"user","text":"Hello","metadata":{"team_id":"TTEST","channel_id":"CTEST","message_timestamp":"1.000001","reply_thread_timestamp":"1.000001"},"thread_id":"1.000001"})).unwrap();
        Fixture {
            _directory: directory,
            store,
            access,
            actor,
            chat,
            message,
        }
    }

    #[rstest]
    #[tokio::test]
    async fn link_codes_are_single_use_and_bind_only_the_linking_sender(
        #[future(awt)] fixture: Fixture,
    ) {
        let code = fixture
            .store
            .slack_code(&fixture.access, &fixture.chat.id)
            .await
            .unwrap();
        let code = code.command.strip_prefix("lens link ").unwrap();
        fixture
            .store
            .link_slack(code, &fixture.message)
            .await
            .unwrap();
        assert!(
            fixture
                .store
                .link_slack(code, &fixture.message)
                .await
                .is_err()
        );
        assert_eq!(
            fixture
                .store
                .slack_access(&fixture.message)
                .await
                .unwrap()
                .unwrap()
                .1,
            fixture.chat.id
        );
        let mut other = fixture.message.clone();
        other.sender.id = "UOTHER".into();
        assert!(fixture.store.slack_access(&other).await.unwrap().is_none());
        fixture
            .store
            .unlink_slack(&fixture.access, &fixture.chat.id)
            .await
            .unwrap();
        assert!(
            fixture
                .store
                .slack_access(&fixture.message)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn expired_codes_cannot_authorize_a_thread(#[future(awt)] fixture: Fixture) {
        let code = fixture
            .store
            .slack_code(&fixture.access, &fixture.chat.id)
            .await
            .unwrap();
        sqlx::query("UPDATE slack_link_codes SET expires_at=0")
            .execute(&fixture.store.0)
            .await
            .unwrap();
        assert!(matches!(
            fixture
                .store
                .link_slack(
                    code.command.strip_prefix("lens link ").unwrap(),
                    &fixture.message
                )
                .await,
            Err(Error::Forbidden)
        ));
        assert!(
            fixture
                .store
                .slack_access(&fixture.message)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn deduplication_and_history_survive_restart(#[future(awt)] fixture: Fixture) {
        let state = SlackState::new(fixture.store.clone());
        assert!(state.claim(&fixture.message.id).await.unwrap());
        state.append(fixture.message.clone()).await.unwrap();
        let reopened = Store::open(&fixture._directory.path().join("state.db"))
            .await
            .unwrap();
        let restarted = SlackState::new(reopened);
        assert!(!restarted.claim(&fixture.message.id).await.unwrap());
        assert_eq!(
            restarted
                .history(&fixture.message.conversation_id, None)
                .await
                .unwrap(),
            vec![fixture.message]
        );
    }

    #[rstest]
    #[case::confirmed(true)]
    #[case::uncertain(false)]
    #[tokio::test]
    async fn proactive_delivery_is_not_duplicated(
        #[future(awt)] fixture: Fixture,
        #[case] confirmed: bool,
    ) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"ok":true,"team_id":"TTEST","user_id":"UBOT","bot_id":"BTEST"}),
            ))
            .mount(&server)
            .await;
        let response = if confirmed {
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"channel":"CTEST","ts":"2.000001","message":{"text":"Found failures"}}))
        } else {
            ResponseTemplate::new(503)
        };
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let mut config = SlackConfig::new("test-bot-token".into(), "test-app-token".into());
        config.api_base_url = format!("{}/", server.uri());
        config.ack_emoji = None;
        let state = Arc::new(SlackState::new(fixture.store.clone()));
        let slack = Arc::new(Slack::with_thread_subscriptions(config, state.clone()).unwrap());
        slack.authenticate().await.unwrap();
        let bridge = Bridge {
            slack,
            app: Arc::new(AgentChat::with_state(state)),
        };
        let code = fixture
            .store
            .slack_code(&fixture.access, &fixture.chat.id)
            .await
            .unwrap();
        fixture
            .store
            .link_slack(
                code.command.strip_prefix("lens link ").unwrap(),
                &fixture.message,
            )
            .await
            .unwrap();
        let request = || SlackMessage {
            id: "report-update-1".into(),
            conversation_id: fixture.chat.id.clone(),
            text: "Found failures".into(),
        };
        assert_eq!(
            bridge
                .send(&fixture.store, &fixture.actor, request())
                .await
                .is_ok(),
            confirmed
        );
        let retry = bridge
            .send(&fixture.store, &fixture.actor, request())
            .await
            .unwrap();
        assert_eq!(
            retry["status"],
            if confirmed { "sent" } else { "uncertain" }
        );
        server.verify().await;
    }
}
