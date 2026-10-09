use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};

use chrono::Utc;
use http::HeaderMap;
use lens_auth::{
    IngestionError, StoreError,
    ingestion::{Ingestion, IngestionRepository},
};
use lens_contract::ingestion::{IngestionKey, IngestionTenant};
use litellm_lens::{Error, auth::Credentials, local_credentials::LocalCredentials};
use rstest::{fixture, rstest};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

const TOKEN: &str = "local-ingestion-test-token";

#[derive(Clone, Default)]
struct Repository {
    records: Arc<Mutex<Vec<IngestionKey>>>,
    hold: Arc<AtomicBool>,
    unavailable: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl IngestionRepository for Repository {
    async fn list(&self) -> Result<Vec<IngestionKey>, IngestionError> {
        if self.unavailable.load(Ordering::Acquire) {
            return Err(StoreError::Unavailable(
                std::io::Error::other("test storage failure").into(),
            )
            .into());
        }
        let records = self.records.lock().await.clone();
        if self.hold.swap(false, Ordering::AcqRel) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(records)
    }

    async fn insert(&self, key: &IngestionKey) -> Result<(), IngestionError> {
        self.records.lock().await.push(key.clone());
        Ok(())
    }

    async fn revoke(&self, id: &str) -> Result<(), IngestionError> {
        self.records.lock().await.retain(|key| key.id != id);
        Ok(())
    }
}

#[fixture]
async fn repository() -> Repository {
    let repository = Repository::default();
    repository
        .insert(&IngestionKey {
            id: "trace-key".into(),
            name: "test".into(),
            created_at: Utc::now(),
            expires_at: None,
            tenant: IngestionTenant {
                team_id: "test-team".into(),
                user_id: "test-user".into(),
                org_id: "test-org".into(),
                api_key_hash: format!("{:x}", Sha256::digest(TOKEN)),
            },
        })
        .await
        .unwrap();
    repository
}

fn headers() -> HeaderMap {
    HeaderMap::from_iter([(
        http::header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    )])
}

#[rstest]
#[tokio::test]
async fn local_snapshots_apply_ownership_and_revocation(#[future(awt)] repository: Repository) {
    let credentials = Arc::new(Credentials::default());
    let local = LocalCredentials::new(Ingestion(repository.clone()), credentials.clone());
    assert!(local.synchronize().await.unwrap());
    assert!(credentials.ready());
    let tenant = credentials.tenant(&headers()).unwrap();
    assert_eq!(tenant.user_id, "test-user");
    assert_eq!(tenant.team_id, "test-team");
    assert_eq!(tenant.org_id, "test-org");
    assert_eq!(tenant.api_key_hash, format!("{:x}", Sha256::digest(TOKEN)));
    repository.revoke("trace-key").await.unwrap();
    assert!(local.synchronize().await.unwrap());
    assert!(matches!(
        credentials.tenant(&headers()),
        Err(Error::Unauthorized)
    ));
}

#[rstest]
#[tokio::test]
async fn failed_first_refresh_keeps_ingestion_unavailable(#[future(awt)] repository: Repository) {
    repository.unavailable.store(true, Ordering::Release);
    let credentials = Arc::new(Credentials::default());
    let local = LocalCredentials::new(Ingestion(repository), credentials.clone());
    assert!(matches!(
        local.synchronize().await,
        Err(IngestionError::Store(_))
    ));
    assert!(!credentials.ready());
    assert!(matches!(
        credentials.tenant(&headers()),
        Err(Error::Unavailable)
    ));
}

#[rstest]
#[tokio::test]
async fn revoke_waits_for_older_refresh_and_cannot_reinstall_old_key(
    #[future(awt)] repository: Repository,
) {
    let credentials = Arc::new(Credentials::default());
    let local = Arc::new(LocalCredentials::new(
        Ingestion(repository.clone()),
        credentials.clone(),
    ));
    repository.hold.store(true, Ordering::Release);
    let background = tokio::spawn({
        let local = local.clone();
        async move { local.synchronize().await }
    });
    repository.entered.notified().await;
    repository.revoke("trace-key").await.unwrap();
    let revoked = local.synchronize();
    tokio::pin!(revoked);
    assert!(matches!(futures_util::poll!(&mut revoked), Poll::Pending));
    repository.release.notify_one();
    assert!(background.await.unwrap().unwrap());
    assert!(revoked.await.unwrap());
    assert!(matches!(
        credentials.tenant(&headers()),
        Err(Error::Unauthorized)
    ));
}
