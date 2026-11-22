mod datasets {
    pub mod support;
}

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use datasets::support::{Database, GATEWAY, SLACK, database};
use futures_util::StreamExt;
use lens_contract::auth::{Identity, Role};
use reqwest::{Client, Method};
use rstest::rstest;
use serde_json::json;

const ADMIN_PREFIX: &str = "/lens/admin/";
const METHODS: [Method; 5] = [
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
];
const ROLES: [Role; 6] = [
    Role::InternalUser,
    Role::InternalUserViewer,
    Role::OrgAdmin,
    Role::Team,
    Role::Customer,
    Role::SlackInvestigator,
];

fn sources(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .flat_map(|path| match path.is_dir() {
            true => sources(&path),
            false if path.extension().is_some_and(|kind| kind == "rs") => vec![path],
            false => Vec::new(),
        })
        .collect()
}

fn addresses() -> BTreeSet<String> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    ["server/src", "lens/src"]
        .iter()
        .flat_map(|source| sources(&crates.join(source)))
        .flat_map(|path| {
            std::fs::read_to_string(path)
                .unwrap()
                .split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .map(|literal| {
            literal
                .replace("{prefix}", "")
                .replace("{{", "{")
                .replace("}}", "}")
        })
        .filter(|literal| literal.starts_with('/') && !literal.contains(char::is_whitespace))
        .flat_map(|path| {
            let path = path
                .split('/')
                .map(|part| if part.starts_with('{') { "probe" } else { part })
                .collect::<Vec<_>>()
                .join("/");
            [
                path.clone(),
                format!("/lens{path}"),
                format!("/lens/admin{path}"),
            ]
        })
        .filter(|path| path.starts_with("/lens"))
        .collect()
}

fn token(role: &Role) -> String {
    if *role == Role::SlackInvestigator {
        return SLACK.into();
    }
    let now = chrono::Utc::now().timestamp();
    let identity = Identity {
        user_role: *role,
        user_id: Some("caller".into()),
        team_id: Some("alpha".into()),
        log_team_ids: vec!["alpha".into()],
        ..Identity::default()
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &json!({"iss":"litellm","aud":"litellm-lens","sub":"caller","iat":now,"exp":now+60,"identity":identity}),
        &jsonwebtoken::EncodingKey::from_secret(GATEWAY.as_bytes()),
    )
    .unwrap()
}

async fn refuses(client: &Client, method: &Method, url: &str, role: &Role) -> bool {
    let status = client
        .request(method.clone(), url)
        .bearer_auth(token(role))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    status == 403 || (status == 401 && *role == Role::SlackInvestigator)
}

async fn refused(client: &Client, method: &Method, url: &str) -> bool {
    futures_util::future::join_all(ROLES.iter().map(|role| refuses(client, method, url, role)))
        .await
        .iter()
        .all(|refused| *refused)
}

#[rstest]
#[tokio::test]
async fn routes_no_other_role_can_use_answer_only_under_the_admin_prefix(
    #[future(awt)] database: Database,
) {
    let server = database.serve(true).await;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let probes = addresses()
        .into_iter()
        .flat_map(|path| METHODS.iter().map(move |method| (method, path.clone())));
    let admin_only = futures_util::stream::iter(probes)
        .filter_map(|(method, path)| {
            let client = &client;
            let url = format!("{}{path}", server.url);
            async move {
                refused(client, method, &url)
                    .await
                    .then(|| format!("{method} {path}"))
            }
        })
        .collect::<Vec<_>>()
        .await;
    let outside = admin_only
        .iter()
        .filter(|route| !route.contains(ADMIN_PREFIX))
        .collect::<Vec<_>>();
    assert_eq!(outside, Vec::<&String>::new());
    assert!(admin_only.len() > 40, "{admin_only:?}");
}
