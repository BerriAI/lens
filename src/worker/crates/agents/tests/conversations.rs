use lens_agents::{
    Error,
    conversations::{Access, BotInput, Runtime},
    store::Store,
};
use litellm_traces::query::named::ReadAccessParams;
use rstest::{fixture, rstest};
use tempfile::TempDir;

#[fixture]
fn directory() -> TempDir {
    tempfile::tempdir().unwrap()
}
#[fixture]
fn access() -> Access {
    Access {
        workspace: "global".into(),
        owner: "admin-a".into(),
        scope: ReadAccessParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: vec![],
        },
        writable: true,
    }
}
#[fixture]
fn input() -> BotInput {
    BotInput {
        id: Some(uuid::Uuid::new_v4().to_string()),
        model: None,
        name: "Request reviewer".into(),
        instructions: "Review unsuccessful tasks".into(),
    }
}

#[rstest]
#[case::main(Runtime::Deepagents, false)]
#[case::bot(Runtime::Hermes, true)]
#[tokio::test]
async fn private_main_chats_and_shared_bots(
    directory: TempDir,
    access: Access,
    input: BotInput,
    #[case] runtime: Runtime,
    #[case] shared: bool,
) {
    let store = Store::open(&directory.path().join("state.db"))
        .await
        .unwrap();
    let actor = store.create_agent(&access, &input, runtime).await.unwrap();
    let chat = store
        .create_conversation(&access, &actor.id, "Investigation")
        .await
        .unwrap();
    let other = Access {
        owner: "admin-b".into(),
        ..access
    };
    assert_eq!(store.conversation(&other, &chat.id).await.is_ok(), shared);
    let foreign = Access {
        workspace: "team:other".into(),
        ..other
    };
    assert!(store.conversation(&foreign, &chat.id).await.is_err());
}

#[rstest]
#[tokio::test]
async fn retries_do_not_create_another_bot_chat_or_turn(
    directory: TempDir,
    access: Access,
    input: BotInput,
) {
    let store = Store::open(&directory.path().join("state.db"))
        .await
        .unwrap();
    let first = store
        .create_agent(&access, &input, Runtime::Hermes)
        .await
        .unwrap();
    let again = store
        .create_agent(&access, &input, Runtime::Hermes)
        .await
        .unwrap();
    assert_eq!(first.id, again.id);
    let id = uuid::Uuid::new_v4().to_string();
    let chat = store
        .create_conversation_with_id(&access, &first.id, "Investigation", &id)
        .await
        .unwrap();
    assert_eq!(
        chat.id,
        store
            .create_conversation_with_id(&access, &first.id, "Investigation", &id)
            .await
            .unwrap()
            .id
    );
    assert!(
        store
            .admit(&access, &chat.id, &id, "Check failures")
            .await
            .unwrap()
            .1
    );
    assert!(
        !store
            .admit(&access, &chat.id, &id, "Check failures")
            .await
            .unwrap()
            .1
    );
    assert!(matches!(
        store
            .admit(&access, &chat.id, &id, "Different request")
            .await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(store.turns(&access, &chat.id).await.unwrap().len(), 1);
}

#[rstest]
#[tokio::test]
async fn viewers_cannot_create_bots_or_turns(directory: TempDir, access: Access, input: BotInput) {
    let store = Store::open(&directory.path().join("state.db"))
        .await
        .unwrap();
    let bot = store
        .create_agent(&access, &input, Runtime::Hermes)
        .await
        .unwrap();
    let chat = store
        .conversations(&access, &bot.id)
        .await
        .unwrap()
        .remove(0);
    let viewer = Access {
        writable: false,
        ..access
    };
    assert!(matches!(
        store.create_agent(&viewer, &input, Runtime::Hermes).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.set_enabled(&viewer, &bot.id, false).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store
            .admit(&viewer, &chat.id, &uuid::Uuid::new_v4().to_string(), "Run")
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        store.conversation(&viewer, &chat.id).await.unwrap().id,
        chat.id
    );
}

#[rstest]
#[tokio::test]
async fn interrupted_turn_cannot_be_revived_by_late_runtime_events(
    directory: TempDir,
    access: Access,
    input: BotInput,
) {
    let store = Store::open(&directory.path().join("state.db"))
        .await
        .unwrap();
    let bot = store
        .create_agent(&access, &input, Runtime::Hermes)
        .await
        .unwrap();
    let chat = store
        .conversations(&access, &bot.id)
        .await
        .unwrap()
        .remove(0);
    let (turn, _) = store
        .admit(&access, &chat.id, &uuid::Uuid::new_v4().to_string(), "Run")
        .await
        .unwrap();
    store
        .turn_status(&turn.id, "interrupted", Some("Stopped"))
        .await
        .unwrap();
    store
        .turn_status(&turn.id, "completed", None)
        .await
        .unwrap();
    assert_eq!(
        store.turns(&access, &chat.id).await.unwrap()[0].status,
        "interrupted"
    );
}

#[rstest]
#[case::session(Runtime::Deepagents, "session-model")]
#[case::bot(Runtime::Hermes, "bot-model")]
#[tokio::test]
async fn selected_model_survives_reopen_and_bot_chats_inherit_it(
    directory: TempDir,
    access: Access,
    input: BotInput,
    #[case] runtime: Runtime,
    #[case] expected: &str,
) {
    let path = directory.path().join("models.db");
    let store = Store::open(&path).await.unwrap();
    let input = BotInput {
        model: Some("bot-model".into()),
        ..input
    };
    let agent = store.create_agent(&access, &input, runtime).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let chat = store
        .create_conversation_with_model(
            &access,
            &agent.id,
            "Model choice",
            &id,
            Some("session-model"),
        )
        .await
        .unwrap();
    assert_eq!(chat.model.as_deref(), Some(expected));
    drop(store);
    let reopened = Store::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .conversation(&access, &id)
            .await
            .unwrap()
            .model
            .as_deref(),
        Some(expected)
    );
    assert_eq!(
        reopened
            .agent(&access.workspace, &agent.id)
            .await
            .unwrap()
            .model,
        input.model
    );
}

#[rstest]
#[tokio::test]
async fn model_changes_cannot_reuse_a_session_request(
    directory: TempDir,
    access: Access,
    input: BotInput,
) {
    let store = Store::open(&directory.path().join("models.db"))
        .await
        .unwrap();
    let agent = store
        .create_agent(&access, &input, Runtime::Deepagents)
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    store
        .create_conversation_with_model(&access, &agent.id, "Model choice", &id, Some("model-one"))
        .await
        .unwrap();
    assert!(matches!(
        store
            .create_conversation_with_model(
                &access,
                &agent.id,
                "Model choice",
                &id,
                Some("model-two")
            )
            .await,
        Err(Error::Conflict(_))
    ));
}
