mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use lens_contract::auth::{Identity, Role};
use lens_server::service::{ServiceConnection, ServiceProvider, ServiceStatus};
use rstest::rstest;
use serde_json::{Value, json};
use support::{Database, database};

struct Service;

impl ServiceProvider for Service {
    fn connection(&self) -> ServiceConnection {
        ServiceConnection {
            url: "https://lens.test/lens-ingest".into(),
            connected: true,
            status: ServiceStatus {
                storage_ready: true,
                credentials_ready: true,
                release: "example-release".into(),
                protocol_version: 1,
                public_contract: 1,
            },
            configured: true,
            release: "example-release".into(),
        }
    }
}

#[rstest]
#[case::admin(Some(Role::ProxyAdmin), 200)]
#[case::viewer(Some(Role::ProxyAdminViewer), 200)]
#[case::user(Some(Role::InternalUser), 200)]
#[case::anonymous(None, 401)]
#[tokio::test]
async fn service_connection_is_available_to_authenticated_users(
    #[future(awt)] database: Database,
    #[case] role: Option<Role>,
    #[case] status: u16,
) {
    let server = database
        .serve_router(false, |auth| lens_server::service::router(auth, Service))
        .await;
    let request = server.client.get(server.url.join("/lens/service").unwrap());
    let request = if let Some(role) = role {
        request.bearer_auth(identity::delegated(Identity {
            user_role: role,
            user_id: Some("user".into()),
            ..Identity::default()
        }))
    } else {
        request
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), status);
    let body: Value = response.json().await.unwrap();
    if status == 200 {
        assert_eq!(
            body,
            json!({"url":"https://lens.test/lens-ingest","connected":true,"status":{"storage_ready":true,"credentials_ready":true,"release":"example-release","protocol_version":1,"public_contract":1},"configured":true,"release":"example-release"})
        );
    } else {
        assert_eq!(body, json!({"detail":"Sign in to Lens"}));
    }
}
