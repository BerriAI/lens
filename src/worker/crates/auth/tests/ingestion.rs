#[path = "ingestion/support.rs"]
mod support;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use lens_auth::{
    IngestionError, StoreError,
    ingestion::{Ingestion, IngestionRepository},
};
use lens_contract::{
    auth::{Identity, Role},
    ingestion::{IngestionKey, IngestionKeyRequest},
};
use rstest::rstest;
use sha2::{Digest, Sha256};
use support::{Memory, admin, ingestion, now};

#[rstest]
#[tokio::test]
async fn creation_returns_a_secret_once_and_persists_owner_and_digest(
    ingestion: Ingestion<Memory>,
    admin: Identity,
    now: DateTime<Utc>,
) {
    let request = IngestionKeyRequest {
        name: "Agent".into(),
        team_id: "requested-team".into(),
        expires_at: Some(now + Duration::minutes(5)),
    };
    let created = ingestion.create(&admin, request, now).await.unwrap();
    let entropy = created
        .key
        .strip_prefix(&format!("lens-trace-{}-", now.timestamp()))
        .unwrap();
    assert_eq!(URL_SAFE_NO_PAD.decode(entropy).unwrap().len(), 40);
    assert_eq!(created.record.tenant.user_id, "owner");
    assert_eq!(created.record.tenant.team_id, "requested-team");
    assert_eq!(created.record.tenant.org_id, "");
    assert_eq!(created.record.name, "Agent");
    assert_eq!(created.record.created_at, now);
    assert_eq!(
        created.record.expires_at,
        Some((now + Duration::minutes(5)).timestamp())
    );
    assert_eq!(
        created.record.tenant.api_key_hash,
        format!("{:x}", Sha256::digest(created.key.as_bytes()))
    );
    assert_eq!(
        uuid::Uuid::parse_str(&created.record.id)
            .unwrap()
            .get_version_num(),
        4
    );
    assert!(!created.active);
    let stored = ingestion.list(&admin).await.unwrap();
    assert_eq!(stored, vec![created.record]);
    assert!(
        !serde_json::to_string(&stored)
            .unwrap()
            .contains(&created.key)
    );
}

#[rstest]
#[tokio::test]
async fn default_requests_create_independent_keys_without_expiry(
    ingestion: Ingestion<Memory>,
    now: DateTime<Utc>,
) {
    let admin = Identity {
        user_id: None,
        ..admin()
    };
    let first = ingestion
        .create(&admin, IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    let second = ingestion
        .create(&admin, IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    assert_ne!(first.key, second.key);
    assert_ne!(first.record.id, second.record.id);
    assert_ne!(
        first.record.tenant.api_key_hash,
        second.record.tenant.api_key_hash
    );
    assert_eq!(first.record.name, "Agent tracing");
    assert_eq!(first.record.expires_at, None);
    assert_eq!(first.record.tenant.team_id, "");
    assert_eq!(first.record.tenant.user_id, "");
    assert_eq!(ingestion.list(&admin).await.unwrap().len(), 2);
}

#[rstest]
#[case::past(Duration::seconds(-1))]
#[case::equal(Duration::zero())]
#[case::negative_epoch(Duration::days(-100_000))]
#[tokio::test]
async fn nonfuture_expiry_does_not_store_a_key(
    ingestion: Ingestion<Memory>,
    admin: Identity,
    now: DateTime<Utc>,
    #[case] offset: Duration,
) {
    let request = IngestionKeyRequest {
        expires_at: Some(now + offset),
        ..IngestionKeyRequest::default()
    };
    assert!(matches!(
        ingestion.create(&admin, request, now).await,
        Err(IngestionError::InvalidExpiry)
    ));
    assert!(ingestion.list(&admin).await.unwrap().is_empty());
}

#[rstest]
#[case::empty_name("".into(), "".into(), false)]
#[case::long_name("a".repeat(129), "".into(), false)]
#[case::max_name("λ".repeat(128), "".into(), true)]
#[case::long_team("Agent".into(), "x".repeat(257), false)]
#[case::max_team("Agent".into(), "λ".repeat(256), true)]
#[tokio::test]
async fn request_lengths_count_unicode_characters(
    ingestion: Ingestion<Memory>,
    admin: Identity,
    now: DateTime<Utc>,
    #[case] name: String,
    #[case] team_id: String,
    #[case] accepted: bool,
) {
    let request = IngestionKeyRequest {
        name,
        team_id,
        expires_at: None,
    };
    let result = ingestion.create(&admin, request, now).await;
    assert_eq!(result.is_ok(), accepted);
    assert_eq!(
        ingestion.list(&admin).await.unwrap().len(),
        usize::from(accepted)
    );
    if !accepted {
        assert!(matches!(result, Err(IngestionError::InvalidLength { .. })));
    }
}

#[rstest]
#[case::viewer(Role::ProxyAdminViewer)]
#[case::org_admin(Role::OrgAdmin)]
#[case::user(Role::InternalUser)]
#[case::user_viewer(Role::InternalUserViewer)]
#[case::team(Role::Team)]
#[case::customer(Role::Customer)]
#[tokio::test]
async fn other_roles_cannot_create_or_revoke_keys(
    ingestion: Ingestion<Memory>,
    now: DateTime<Utc>,
    #[case] user_role: Role,
) {
    let created = ingestion
        .create(&admin(), IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    let identity = Identity {
        user_role,
        ..admin()
    };
    assert!(matches!(
        ingestion
            .create(&identity, IngestionKeyRequest::default(), now)
            .await,
        Err(IngestionError::Forbidden(
            "Only proxy admins can configure or run Lens"
        ))
    ));
    assert!(matches!(
        ingestion.revoke(&identity, &created.record.id).await,
        Err(IngestionError::Forbidden(
            "Only proxy admins can configure or run Lens"
        ))
    ));
    assert_eq!(
        ingestion.list(&admin()).await.unwrap(),
        vec![created.record]
    );
}

#[rstest]
#[case::admin(Role::ProxyAdmin, true)]
#[case::viewer(Role::ProxyAdminViewer, true)]
#[case::org_admin(Role::OrgAdmin, false)]
#[case::user(Role::InternalUser, false)]
#[case::user_viewer(Role::InternalUserViewer, false)]
#[case::team(Role::Team, false)]
#[case::customer(Role::Customer, false)]
#[tokio::test]
async fn only_proxy_administrators_can_list_keys(
    ingestion: Ingestion<Memory>,
    now: DateTime<Utc>,
    #[case] user_role: Role,
    #[case] accepted: bool,
) {
    let created = ingestion
        .create(&admin(), IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    let identity = Identity {
        user_role,
        ..admin()
    };
    let result = ingestion.list(&identity).await;
    if accepted {
        assert_eq!(result.unwrap(), vec![created.record]);
    } else {
        assert!(matches!(
            result,
            Err(IngestionError::Forbidden(
                "Lens requires proxy administrator access"
            ))
        ));
    }
}

#[rstest]
#[case::never(None, true)]
#[case::future(Some(1), true)]
#[case::equal(Some(0), false)]
#[case::past(Some(-1), false)]
#[case::negative_epoch(Some(-10_000_000_000), false)]
#[tokio::test]
async fn snapshot_excludes_expired_keys_without_deleting_them(
    ingestion: Ingestion<Memory>,
    admin: Identity,
    now: DateTime<Utc>,
    #[case] offset: Option<i64>,
    #[case] active: bool,
) {
    let mut key = ingestion
        .create(&admin, IngestionKeyRequest::default(), now)
        .await
        .unwrap()
        .record;
    key.expires_at = offset.map(|offset| now.timestamp() + offset);
    *ingestion.0.0.lock().unwrap() = vec![key.clone()];
    let snapshot = ingestion.current_snapshot(now).await.unwrap();
    assert_eq!(snapshot.issued_at, now.timestamp());
    assert_eq!(snapshot.keys.len(), usize::from(active));
    if active {
        assert_eq!(snapshot.keys[0].token_hash, key.tenant.api_key_hash);
        assert_eq!(snapshot.keys[0].tenant, key.tenant);
        assert_eq!(snapshot.keys[0].expires_at, key.expires_at);
    }
    assert_eq!(ingestion.list(&admin).await.unwrap(), vec![key]);
}

#[rstest]
#[tokio::test]
async fn subsecond_expiry_preserves_python_integer_boundary(
    ingestion: Ingestion<Memory>,
    admin: Identity,
    now: DateTime<Utc>,
) {
    let request = IngestionKeyRequest {
        expires_at: Some(now + Duration::microseconds(1)),
        ..IngestionKeyRequest::default()
    };
    let created = ingestion.create(&admin, request, now).await.unwrap();
    assert_eq!(created.record.expires_at, Some(now.timestamp()));
    assert!(
        ingestion
            .current_snapshot(now)
            .await
            .unwrap()
            .keys
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn revocation_removes_only_the_selected_key_and_is_idempotent(
    ingestion: Ingestion<Memory>,
    admin: Identity,
    now: DateTime<Utc>,
) {
    let first = ingestion
        .create(&admin, IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    let second = ingestion
        .create(&admin, IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    ingestion.revoke(&admin, &first.record.id).await.unwrap();
    ingestion.revoke(&admin, &first.record.id).await.unwrap();
    assert_eq!(
        ingestion.list(&admin).await.unwrap(),
        vec![second.record.clone()]
    );
    let snapshot = ingestion.current_snapshot(now).await.unwrap();
    assert_eq!(snapshot.keys.len(), 1);
    assert_eq!(snapshot.keys[0].tenant, second.record.tenant);
}

struct Unavailable;

impl IngestionRepository for Unavailable {
    async fn list(&self) -> Result<Vec<IngestionKey>, IngestionError> {
        Err(StoreError::Conflict.into())
    }
    async fn insert(&self, _: &IngestionKey) -> Result<(), IngestionError> {
        Err(StoreError::Conflict.into())
    }
    async fn revoke(&self, _: &str) -> Result<(), IngestionError> {
        Err(StoreError::Conflict.into())
    }
}

#[rstest]
#[tokio::test]
async fn persistence_failures_do_not_report_success(admin: Identity, now: DateTime<Utc>) {
    let ingestion = Ingestion(Unavailable);
    assert!(matches!(
        ingestion
            .create(&admin, IngestionKeyRequest::default(), now)
            .await,
        Err(IngestionError::Store(StoreError::Conflict))
    ));
    assert!(matches!(
        ingestion.list(&admin).await,
        Err(IngestionError::Store(StoreError::Conflict))
    ));
    assert!(matches!(
        ingestion.revoke(&admin, "missing").await,
        Err(IngestionError::Store(StoreError::Conflict))
    ));
    assert!(matches!(
        ingestion.current_snapshot(now).await,
        Err(IngestionError::Store(StoreError::Conflict))
    ));
}
