mod support;

use chrono::{DateTime, Duration, Utc};
use lens_auth::{
    Authentication, Error, GatewayConnection, ReadScope, Session, SessionId, Settings, StoreError,
    local_admin, session_view, trace_read_scope,
};
use lens_contract::auth::{Identity, Role};
use rstest::rstest;
use support::{ADMIN, Memory, ORIGIN, authentication, bearer, cookie, now};

#[rstest]
#[case::plain("http://lens.test", false)]
#[case::tls("https://lens.test", true)]
#[case::trailing_slash("https://lens.test/", true)]
fn cookie_security_follows_configuration(#[case] url: &str, #[case] secure: bool) {
    assert_eq!(
        Settings::new(ADMIN, None, url).unwrap().secure_cookie(),
        secure
    );
}

#[rstest]
#[case::weak_token("short", ORIGIN)]
#[case::empty_token("", ORIGIN)]
#[case::bad_url(ADMIN, "bad")]
#[case::wrong_scheme(ADMIN, "ftp://lens.test")]
#[case::missing_host(ADMIN, "https://")]
#[case::username(ADMIN, "https://user@lens.test")]
#[case::password(ADMIN, "https://:password@lens.test")]
#[case::path(ADMIN, "https://lens.test/ui")]
#[case::query(ADMIN, "https://lens.test?q=1")]
#[case::fragment(ADMIN, "https://lens.test#ui")]
fn unsafe_settings_fail(#[case] token: &str, #[case] url: &str) {
    assert!(matches!(
        Settings::new(token, None, url),
        Err(Error::Configuration(_))
    ));
}

#[rstest]
#[case::minimum("a".repeat(32))]
#[case::multibyte("λ".repeat(32))]
fn minimum_secret_is_measured_in_characters(#[case] token: String) {
    assert!(Settings::new(&token, None, ORIGIN).is_ok());
    assert!(Settings::new(&token.chars().take(31).collect::<String>(), None, ORIGIN).is_err());
}

#[rstest]
#[case::disabled(None, true)]
#[case::empty(Some(String::new()), false)]
#[case::short(Some("a".repeat(31)), false)]
#[case::minimum(Some("a".repeat(32)), true)]
#[case::multibyte_short(Some("λ".repeat(31)), false)]
#[case::multibyte_minimum(Some("λ".repeat(32)), true)]
fn gateway_delegation_requires_a_strong_configured_secret(
    #[case] secret: Option<String>,
    #[case] valid: bool,
) {
    let settings = Settings::new(ADMIN, secret, ORIGIN);
    assert_eq!(settings.is_ok(), valid);
    if !valid {
        assert!(matches!(
            settings,
            Err(Error::Configuration(
                "gateway secret must contain at least 32 characters"
            ))
        ));
    }
}

#[rstest]
#[case::admin(ADMIN)]
#[case::short("short")]
fn slack_credential_cannot_reuse_admin_or_weak_keys(#[case] key: &str) {
    assert!(matches!(
        Settings::new(ADMIN, None, ORIGIN)
            .unwrap()
            .with_slack_investigator(key, "docs-team"),
        Err(Error::Configuration(_))
    ));
}

#[rstest]
#[tokio::test]
async fn slack_credential_only_authenticates_on_opted_in_routes(now: DateTime<Utc>) {
    const KEY: &str = "slack-investigator-test-token-only";
    let authentication = Authentication {
        settings: Settings::new(ADMIN, None, ORIGIN)
            .unwrap()
            .with_slack_investigator(KEY, "docs-team")
            .unwrap(),
        sessions: Memory::default(),
    };
    let header = format!("Bearer {KEY}");
    assert!(matches!(
        authentication.authenticate(bearer(&header), now).await,
        Err(Error::Unauthorized(_))
    ));
    let identity = authentication
        .authenticate_slack(bearer(&header), now)
        .await
        .unwrap();
    assert_eq!(
        trace_read_scope(&identity),
        Some(ReadScope::OwnedRows {
            user_id: String::new(),
            team_ids: vec!["docs-team".into()],
        }),
    );
}

#[rstest]
#[tokio::test]
async fn sign_in_stores_a_hash_with_eight_hour_expiry(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    let signed_in = authentication
        .sign_in(ADMIN, "new-session", now)
        .await
        .unwrap();
    assert_eq!(signed_in.view.user_id, "lens-admin");
    assert_eq!(signed_in.view.user_role, Role::ProxyAdmin);
    assert_eq!(signed_in.lifetime_seconds, 8 * 60 * 60);
    let values = authentication.sessions.values.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(
        values.get(SessionId::for_token("new-session").as_str()),
        Some(&Session {
            expires_at: now + Duration::hours(8),
            identity: None,
        })
    );
    assert!(!values.contains_key("new-session"));
    assert_eq!(
        SessionId::for_token("abc").as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[rstest]
#[case::wrong("wrong")]
#[case::empty("")]
#[case::same_length("parity-admin-token-32-characters-LONG")]
#[tokio::test]
async fn failed_sign_in_does_not_write(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] token: &str,
) {
    assert_eq!(
        authentication
            .sign_in(token, "session", now)
            .await
            .unwrap_err()
            .to_string(),
        "Invalid Lens setup token"
    );
    assert!(authentication.sessions.values.lock().unwrap().is_empty());
}

#[rstest]
#[case::just_before(-1, true)]
#[case::at_expiry(0, false)]
#[case::after(1, false)]
#[tokio::test]
async fn cookie_expiry_is_strict(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] microseconds: i64,
    #[case] active: bool,
) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    let result = authentication
        .authenticate(
            cookie("active"),
            now + Duration::hours(8) + Duration::microseconds(microseconds),
        )
        .await;
    if active {
        assert_eq!(result.unwrap(), local_admin());
    } else {
        assert_eq!(result.unwrap_err().to_string(), "Lens session has expired");
    }
}

#[rstest]
#[case::get("GET", None, true)]
#[case::head("HEAD", None, true)]
#[case::options("OPTIONS", None, true)]
#[case::delete("DELETE", Some(ORIGIN), true)]
#[case::post("POST", Some(ORIGIN), true)]
#[case::missing("DELETE", None, false)]
#[case::wrong("POST", Some("https://other.test"), false)]
#[case::same_host_wrong_scheme("PUT", Some("http://lens.test"), false)]
#[case::trailing_path("PATCH", Some("https://lens.test/"), false)]
#[tokio::test]
async fn cookie_writes_require_exact_origin(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] method: &str,
    #[case] origin: Option<&str>,
    #[case] allowed: bool,
) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    let result = authentication
        .authenticate(
            lens_auth::Credentials {
                method,
                origin,
                ..cookie("active")
            },
            now,
        )
        .await;
    if allowed {
        assert_eq!(result.unwrap(), local_admin());
    } else {
        assert!(matches!(result, Err(Error::OriginMismatch)));
    }
}

#[rstest]
#[case::uppercase_authority("http://LENS.test:80", "http://LENS.test:80", true)]
#[case::preserve_default_port("http://lens.test:80", "http://lens.test", false)]
#[tokio::test]
async fn configured_origin_retains_authority_spelling(
    now: DateTime<Utc>,
    #[case] url: &str,
    #[case] origin: &str,
    #[case] allowed: bool,
) {
    let auth = Authentication {
        settings: Settings::new(ADMIN, None, url).unwrap(),
        sessions: Memory::default(),
    };
    auth.sign_in(ADMIN, "active", now).await.unwrap();
    let result = auth
        .authenticate(
            lens_auth::Credentials {
                method: "DELETE",
                origin: Some(origin),
                ..cookie("active")
            },
            now,
        )
        .await;
    assert_eq!(result.is_ok(), allowed);
}

#[rstest]
#[case::basic("Basic x", "Use a Lens bearer credential")]
#[case::missing_token("Bearer", "Use a Lens bearer credential")]
#[case::empty_token("Bearer ", "Use a Lens bearer credential")]
#[case::extra_space("Bearer  invalid", "Invalid or expired gateway identity")]
#[case::wrong_bearer("Bearer invalid", "Invalid or expired gateway identity")]
#[case::tab("Bearer\tinvalid", "Use a Lens bearer credential")]
#[tokio::test]
async fn invalid_authorization_never_falls_back_to_cookie(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] header: &str,
    #[case] message: &str,
) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    assert_eq!(
        authentication
            .authenticate(bearer(header), now)
            .await
            .unwrap_err()
            .to_string(),
        message
    );
}

#[rstest]
#[case::bearer("Bearer")]
#[case::mixed_case("bEaReR")]
#[tokio::test]
async fn bearer_admin_works_without_storage_or_origin(now: DateTime<Utc>, #[case] scheme: &str) {
    let auth = Authentication {
        settings: Settings::new(ADMIN, None, ORIGIN).unwrap(),
        sessions: Memory {
            unavailable: true,
            ..Memory::default()
        },
    };
    assert_eq!(
        auth.authenticate(bearer(&format!("{scheme} {ADMIN}")), now)
            .await
            .unwrap(),
        local_admin()
    );
    assert_eq!(
        auth.authenticate(bearer("Bearer wrong"), now)
            .await
            .unwrap_err()
            .to_string(),
        "Invalid Lens credential"
    );
}

#[rstest]
#[tokio::test]
async fn empty_authorization_uses_cookie_and_missing_cookie_requires_sign_in(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    assert_eq!(
        authentication
            .authenticate(
                lens_auth::Credentials {
                    authorization: Some(""),
                    ..cookie("active")
                },
                now
            )
            .await
            .unwrap(),
        local_admin()
    );
    assert_eq!(
        authentication
            .authenticate(
                lens_auth::Credentials {
                    session: None,
                    ..cookie("active")
                },
                now
            )
            .await
            .unwrap_err()
            .to_string(),
        "Sign in to Lens"
    );
    assert_eq!(
        authentication
            .authenticate(cookie("missing"), now)
            .await
            .unwrap_err()
            .to_string(),
        "Lens session has expired"
    );
}

#[rstest]
#[tokio::test]
async fn logout_revokes_only_after_authorization(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    let denied = lens_auth::Credentials {
        method: "DELETE",
        origin: Some("https://evil.test"),
        ..cookie("active")
    };
    assert!(matches!(
        authentication.sign_out(denied, now).await,
        Err(Error::OriginMismatch)
    ));
    assert!(
        authentication
            .authenticate(cookie("active"), now)
            .await
            .is_ok()
    );
    authentication
        .sign_out(
            lens_auth::Credentials {
                method: "DELETE",
                origin: Some(ORIGIN),
                ..cookie("active")
            },
            now,
        )
        .await
        .unwrap();
    assert!(
        authentication
            .authenticate(cookie("active"), now)
            .await
            .is_err()
    );
}

#[rstest]
#[case::with_cookie(Some("active"))]
#[case::without_cookie(None)]
#[tokio::test]
async fn bearer_logout_revokes_cookie_if_present(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] session: Option<&str>,
) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    authentication
        .sign_out(
            lens_auth::Credentials {
                session,
                ..bearer(&format!("Bearer {ADMIN}"))
            },
            now,
        )
        .await
        .unwrap();
    assert_eq!(
        authentication
            .authenticate(cookie("active"), now)
            .await
            .is_ok(),
        session.is_none()
    );
}

#[rstest]
#[tokio::test]
async fn repository_failures_propagate(authentication: Authentication<Memory>, now: DateTime<Utc>) {
    authentication.sign_in(ADMIN, "active", now).await.unwrap();
    assert!(matches!(
        authentication.sign_in(ADMIN, "active", now).await,
        Err(Error::Store(StoreError::Conflict))
    ));
    let auth = Authentication {
        settings: authentication.settings,
        sessions: Memory {
            unavailable: true,
            ..Memory::default()
        },
    };
    assert!(matches!(
        auth.sign_in(ADMIN, "new", now).await,
        Err(Error::Store(StoreError::Unavailable(_)))
    ));
    assert!(matches!(
        auth.authenticate(cookie("active"), now).await,
        Err(Error::Store(StoreError::Unavailable(_)))
    ));
    assert!(matches!(
        auth.sign_out(bearer(&format!("Bearer {ADMIN}")), now).await,
        Err(Error::Store(StoreError::Unavailable(_)))
    ));
}

#[rstest]
#[case::admin(Role::ProxyAdmin, None, Some(ReadScope::AllRows))]
#[case::viewer(Role::ProxyAdminViewer, None, Some(ReadScope::AllRows))]
#[case::unidentified(Role::InternalUser, None, None)]
#[case::empty_user(Role::Team, Some(""), None)]
#[case::owned(Role::InternalUser, Some("user"), Some(ReadScope::OwnedRows { user_id: "user".into(), team_ids: vec!["alpha".into(), "beta".into()] }))]
fn read_scope_preserves_authorized_teams(
    #[case] role: Role,
    #[case] user: Option<&str>,
    #[case] scope: Option<ReadScope>,
) {
    let identity = Identity {
        user_role: role,
        user_id: user.map(str::to_owned),
        log_team_ids: vec!["alpha".into(), "beta".into()],
        ..Identity::default()
    };
    assert_eq!(trace_read_scope(&identity), scope);
    let view = session_view(identity);
    assert_eq!(view.user_id, user.unwrap_or_default());
    assert_eq!(view.user_role, role);
}

fn jwt(claims: &serde_json::Value, algorithm: jsonwebtoken::Algorithm, secret: &str) -> String {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(algorithm),
        claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

fn claims(now: DateTime<Utc>) -> serde_json::Value {
    serde_json::json!({ "iss": "litellm", "aud": "litellm-lens", "sub": "user", "iat": now.timestamp(), "exp": now.timestamp() + 30,
        "identity": { "user_id": "user", "team_id": "alpha", "log_team_ids": ["alpha", "beta"] } })
}

#[rstest]
#[case::valid(0, 30, true)]
#[case::maximum_lifetime(0, 60, true)]
#[case::too_long(0, 61, false)]
#[case::small_clock_skew(1, 31, true)]
#[case::clock_skew_boundary(5, 35, true)]
#[case::excessive_clock_skew(6, 36, false)]
#[case::zero_lifetime_with_clock_skew(5, 5, false)]
#[case::negative_lifetime_with_clock_skew(5, 4, false)]
#[case::expiration_boundary(-30, 0, false)]
#[case::expired(-60, -1, false)]
#[case::previous_second(-1, 30, true)]
#[tokio::test]
async fn gateway_time_boundaries(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] issued: i64,
    #[case] expires: i64,
    #[case] valid: bool,
) {
    let mut claims = claims(now);
    claims["iat"] = serde_json::json!(now.timestamp() + issued);
    claims["exp"] = serde_json::json!(now.timestamp() + expires);
    let token = jwt(&claims, jsonwebtoken::Algorithm::HS256, support::SECRET);
    let result = authentication
        .authenticate(bearer(&format!("Bearer {token}")), now)
        .await;
    assert_eq!(result.is_ok(), valid, "{result:?}");
    if valid {
        let identity = result.unwrap();
        assert_eq!(identity.user_id.as_deref(), Some("user"));
        assert_eq!(identity.team_id.as_deref(), Some("alpha"));
        assert_eq!(identity.user_role, Role::InternalUser);
        assert_eq!(identity.log_team_ids, ["alpha", "beta"]);
    }
}

#[rstest]
#[case::issuer("iss", serde_json::json!("other"), "Invalid or expired gateway identity")]
#[case::audience("aud", serde_json::json!("other"), "Invalid or expired gateway identity")]
#[case::audience_array("aud", serde_json::json!(["litellm-lens"]), "Invalid or expired gateway identity")]
#[case::subject("sub", serde_json::json!("other"), "Invalid gateway identity scope")]
#[case::null_identity(
    "identity",
    serde_json::Value::Null,
    "Invalid or expired gateway identity"
)]
#[case::unknown_claim("nbf", serde_json::json!(0), "Invalid or expired gateway identity")]
#[case::unknown_identity_field("identity", serde_json::json!({"user_id":"user", "secret":"extra"}), "Invalid or expired gateway identity")]
#[case::unknown_role("identity", serde_json::json!({"user_id":"user", "user_role":"superadmin"}), "Invalid or expired gateway identity")]
#[case::fractional_iat("iat", serde_json::json!(1699999999.5), "Invalid or expired gateway identity")]
#[case::bad_numeric_date("exp", serde_json::json!("tomorrow"), "Invalid or expired gateway identity")]
#[case::overflow("iat", serde_json::json!(-9.223372036854778e18), "Invalid or expired gateway identity")]
#[case::positive_overflow("exp", serde_json::json!(9.223372036854776e18), "Invalid or expired gateway identity")]
#[case::null_date("exp", serde_json::Value::Null, "Invalid or expired gateway identity")]
#[case::boolean_date("iat", serde_json::json!(false), "Invalid gateway identity scope")]
#[tokio::test]
async fn gateway_rejects_invalid_claims(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] field: &str,
    #[case] value: serde_json::Value,
    #[case] message: &str,
) {
    let mut claims = claims(now);
    claims[field] = value;
    let token = jwt(&claims, jsonwebtoken::Algorithm::HS256, support::SECRET);
    assert_eq!(
        authentication
            .authenticate(bearer(&format!("Bearer {token}")), now)
            .await
            .unwrap_err()
            .to_string(),
        message
    );
}

#[rstest]
#[case::iss("iss")]
#[case::aud("aud")]
#[case::sub("sub")]
#[case::iat("iat")]
#[case::exp("exp")]
#[case::identity("identity")]
#[tokio::test]
async fn all_gateway_claims_are_required(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] field: &str,
) {
    let mut claims = claims(now);
    claims.as_object_mut().unwrap().remove(field);
    let token = jwt(&claims, jsonwebtoken::Algorithm::HS256, support::SECRET);
    assert_eq!(
        authentication
            .authenticate(bearer(&format!("Bearer {token}")), now)
            .await
            .unwrap_err()
            .to_string(),
        "Invalid or expired gateway identity"
    );
}

#[rstest]
#[case::integer_strings(serde_json::json!("1700000000"), serde_json::json!("1700000030"))]
#[case::integral_floats(serde_json::json!(1700000000.0), serde_json::json!(1700000030.0))]
#[case::signed_padded_strings(serde_json::json!(" +1700000000 "), serde_json::json!("1700000030"))]
#[tokio::test]
async fn gateway_preserves_integer_coercion(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] issued: serde_json::Value,
    #[case] expires: serde_json::Value,
) {
    let mut claims = claims(now);
    claims["iat"] = issued;
    claims["exp"] = expires;
    let token = jwt(&claims, jsonwebtoken::Algorithm::HS256, support::SECRET);
    assert_eq!(
        authentication
            .authenticate(bearer(&format!("Bearer {token}")), now)
            .await
            .unwrap()
            .user_id
            .as_deref(),
        Some("user")
    );
}

#[rstest]
#[case::user_preferred(serde_json::json!({"user_id":"user", "token":"key"}), "user", true)]
#[case::token_fallback(serde_json::json!({"token":"key"}), "key", true)]
#[case::empty_user_fallback(serde_json::json!({"user_id":"", "token":"key"}), "key", true)]
#[case::no_subject_identity(serde_json::json!({}), "", false)]
#[case::cannot_use_key_when_user_present(serde_json::json!({"user_id":"user", "token":"key"}), "key", false)]
#[tokio::test]
async fn gateway_subject_binding(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] identity: serde_json::Value,
    #[case] subject: &str,
    #[case] valid: bool,
) {
    let mut claims = claims(now);
    claims["identity"] = identity;
    claims["sub"] = serde_json::json!(subject);
    let token = jwt(&claims, jsonwebtoken::Algorithm::HS256, support::SECRET);
    assert_eq!(
        authentication
            .authenticate(bearer(&format!("Bearer {token}")), now)
            .await
            .is_ok(),
        valid
    );
}

#[rstest]
#[case::wrong_secret(jsonwebtoken::Algorithm::HS256, "wrong secret")]
#[case::wrong_algorithm(jsonwebtoken::Algorithm::HS384, support::SECRET)]
#[tokio::test]
async fn gateway_requires_expected_signature(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
    #[case] algorithm: jsonwebtoken::Algorithm,
    #[case] secret: &str,
) {
    let token = jwt(&claims(now), algorithm, secret);
    assert_eq!(
        authentication
            .authenticate(bearer(&format!("Bearer {token}")), now)
            .await
            .unwrap_err()
            .to_string(),
        "Invalid or expired gateway identity"
    );
}

fn login_token(now: DateTime<Utc>, audience: &str, lifetime: i64) -> String {
    let mut claims = claims(now);
    claims["aud"] = serde_json::json!(audience);
    claims["exp"] = serde_json::json!(now.timestamp() + lifetime);
    jwt(&claims, jsonwebtoken::Algorithm::HS256, support::SECRET)
}

#[rstest]
#[tokio::test]
async fn litellm_login_opens_a_one_hour_session_with_the_gateway_identity(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    let signed_in = authentication
        .sign_in(&login_token(now, "litellm-lens-login", 60), "browser", now)
        .await
        .unwrap();
    assert_eq!(signed_in.view.user_id, "user");
    assert_eq!(signed_in.view.user_role, Role::InternalUser);
    assert_eq!(signed_in.lifetime_seconds, 60 * 60);
    let identity = authentication
        .authenticate(cookie("browser"), now + Duration::minutes(59))
        .await
        .unwrap();
    assert_eq!(
        trace_read_scope(&identity),
        Some(ReadScope::OwnedRows {
            user_id: "user".into(),
            team_ids: vec!["alpha".into(), "beta".into()],
        })
    );
    assert_eq!(
        authentication
            .authenticate(cookie("browser"), now + Duration::hours(1))
            .await
            .unwrap_err()
            .to_string(),
        "Lens session has expired"
    );
}

#[rstest]
#[case::request_token("litellm-lens", 30, support::SECRET)]
#[case::too_long("litellm-lens-login", 61, support::SECRET)]
#[case::expired("litellm-lens-login", 0, support::SECRET)]
#[case::unsigned_by_gateway("litellm-lens-login", 60, "another-gateway-secret-32-characters")]
#[tokio::test]
async fn litellm_login_rejects_tokens_not_issued_for_sign_in(
    now: DateTime<Utc>,
    #[case] audience: &str,
    #[case] lifetime: i64,
    #[case] secret: &str,
) {
    let authentication = Authentication {
        settings: Settings::new(ADMIN, Some(secret.to_owned()), ORIGIN).unwrap(),
        sessions: Memory::default(),
    };
    assert_eq!(
        authentication
            .sign_in(&login_token(now, audience, lifetime), "browser", now)
            .await
            .unwrap_err()
            .to_string(),
        "Invalid Lens setup token"
    );
    assert!(authentication.sessions.values.lock().unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn litellm_login_is_off_without_a_gateway_secret(now: DateTime<Utc>) {
    let authentication = Authentication {
        settings: Settings::new(ADMIN, None, ORIGIN).unwrap(),
        sessions: Memory::default(),
    };
    assert!(
        authentication
            .sign_in(&login_token(now, "litellm-lens-login", 60), "browser", now)
            .await
            .is_err()
    );
}

#[rstest]
#[tokio::test]
async fn sign_in_tokens_cannot_authorize_requests(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    let token = login_token(now, "litellm-lens-login", 60);
    assert!(
        authentication
            .authenticate(bearer(&format!("Bearer {token}")), now)
            .await
            .is_err()
    );
}

#[rstest]
#[case::public_url(
    Some(support::SECRET),
    "https://gateway.test",
    Some("https://gateway.test/lens/login")
)]
#[case::root_path(
    Some(support::SECRET),
    "https://gateway.test/litellm/",
    Some("https://gateway.test/litellm/lens/login")
)]
#[case::openai_base(
    Some(support::SECRET),
    "http://litellm:4000/v1",
    Some("http://litellm:4000/lens/login")
)]
#[case::no_secret(None, "https://gateway.test", None)]
fn litellm_login_link_needs_the_gateway_secret(
    #[case] secret: Option<&str>,
    #[case] gateway: &str,
    #[case] link: Option<&str>,
) {
    let settings = Settings::new(ADMIN, secret.map(str::to_owned), ORIGIN)
        .unwrap()
        .with_gateway_login(gateway)
        .unwrap();
    assert_eq!(settings.gateway_login_url().as_deref(), link);
}

#[rstest]
#[case::not_a_url("gateway")]
#[case::wrong_scheme("ftp://gateway.test")]
#[case::credentials("https://user:pass@gateway.test")]
#[case::query("https://gateway.test?next=evil")]
#[case::fragment("https://gateway.test#x")]
fn litellm_login_rejects_unsafe_gateway_urls(#[case] gateway: &str) {
    assert!(
        Settings::new(ADMIN, Some(support::SECRET.to_owned()), ORIGIN)
            .unwrap()
            .with_gateway_login(gateway)
            .is_err()
    );
}

#[rstest]
#[tokio::test]
async fn litellm_login_token_opens_only_one_session(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    let token = login_token(now, "litellm-lens-login", 60);
    authentication
        .sign_in(&token, "first-browser", now)
        .await
        .unwrap();
    assert_eq!(
        authentication
            .sign_in(&token, "second-browser", now + Duration::seconds(1))
            .await
            .unwrap_err()
            .to_string(),
        "Invalid Lens setup token"
    );
    assert_eq!(
        authentication
            .authenticate(cookie("second-browser"), now + Duration::seconds(2))
            .await
            .unwrap_err()
            .to_string(),
        "Lens session has expired"
    );
}

fn standalone() -> Authentication<Memory> {
    Authentication {
        settings: Settings::new(ADMIN, None, ORIGIN).unwrap(),
        sessions: Memory::default(),
    }
}

fn connection(url: &str, api_key: Option<&str>) -> GatewayConnection {
    GatewayConnection {
        url: url.to_owned(),
        secret: support::SECRET.to_owned(),
        api_key: api_key.map(str::to_owned),
        service_token: Some(SERVICE_TOKEN.to_owned()),
    }
}

const SERVICE_TOKEN: &str = "test-only-service-credential-32-characters";

#[rstest]
#[tokio::test]
async fn connecting_a_gateway_turns_on_litellm_login(now: DateTime<Utc>) {
    let authentication = standalone();
    authentication
        .connect_gateway(ADMIN, connection("http://litellm:4000/v1", None))
        .await
        .unwrap();
    assert_eq!(
        authentication.settings.gateway_login_url().as_deref(),
        Some("http://litellm:4000/lens/login")
    );
    assert!(
        authentication
            .sign_in(&login_token(now, "litellm-lens-login", 60), "browser", now)
            .await
            .is_ok()
    );
}

#[rstest]
#[case::with_key(Some("sk-gateway"), Some(("https://gateway.test".to_owned(), "sk-gateway".to_owned())))]
#[case::login_only(None, None)]
#[tokio::test]
async fn connected_gateway_serves_models_only_with_a_key(
    #[case] api_key: Option<&str>,
    #[case] models: Option<(String, String)>,
) {
    let authentication = standalone();
    authentication
        .connect_gateway(ADMIN, connection("https://gateway.test", api_key))
        .await
        .unwrap();
    assert_eq!(authentication.settings.connected_model_gateway(), models);
}

#[rstest]
#[tokio::test]
async fn a_restarted_lens_loads_the_saved_gateway() {
    let first = standalone();
    first
        .connect_gateway(ADMIN, connection("https://gateway.test", None))
        .await
        .unwrap();
    let restarted = Authentication {
        settings: Settings::new(ADMIN, None, ORIGIN).unwrap(),
        sessions: first.sessions.clone(),
    };
    assert_eq!(restarted.settings.gateway_login_url(), None);
    restarted.refresh_gateway().await.unwrap();
    assert_eq!(
        restarted.settings.gateway_login_url().as_deref(),
        Some("https://gateway.test/lens/login")
    );
}

#[rstest]
#[case::wrong_token(
    "another-admin-token-32-characters-long",
    "https://gateway.test",
    "Invalid Lens admin token"
)]
#[case::not_a_url(ADMIN, "gateway", "Enter the gateway address as an http or https URL")]
#[case::credentials(
    ADMIN,
    "https://user:pass@gateway.test",
    "Enter the gateway address as an http or https URL"
)]
#[tokio::test]
async fn connecting_a_gateway_rejects_bad_requests(
    #[case] token: &str,
    #[case] url: &str,
    #[case] message: &str,
) {
    let authentication = standalone();
    assert_eq!(
        authentication
            .connect_gateway(token, connection(url, None))
            .await
            .unwrap_err()
            .to_string(),
        message
    );
    assert_eq!(authentication.settings.gateway_login_url(), None);
    assert_eq!(*authentication.sessions.gateway.lock().unwrap(), None);
}

#[rstest]
#[tokio::test]
async fn a_gateway_set_in_the_environment_cannot_be_replaced(
    authentication: Authentication<Memory>,
) {
    assert!(matches!(
        authentication
            .connect_gateway(ADMIN, connection("https://other.test", Some("sk-other")))
            .await,
        Err(Error::GatewayLocked)
    ));
    assert_eq!(authentication.settings.connected_model_gateway(), None);
    assert_eq!(*authentication.sessions.gateway.lock().unwrap(), None);
}

#[rstest]
#[case::named(Some("kerry@example.com"), Some("kerry@example.com"))]
#[case::blank(Some("  "), None)]
#[case::unnamed(None, None)]
fn session_view_names_the_user_only_when_the_identity_carries_a_name(
    #[case] name: Option<&str>,
    #[case] shown: Option<&str>,
) {
    let view = lens_auth::session_view(Identity {
        user_id: Some("user".into()),
        user_name: name.map(Into::into),
        ..Identity::default()
    });
    assert_eq!(view.user_id, "user");
    assert_eq!(view.user_name.as_deref(), shown);
}

#[rstest]
#[tokio::test]
async fn another_replica_accepts_a_login_after_a_gateway_is_connected(now: DateTime<Utc>) {
    let first = standalone();
    let replica = Authentication {
        settings: Settings::new(ADMIN, None, ORIGIN).unwrap(),
        sessions: first.sessions.clone(),
    };
    first
        .connect_gateway(ADMIN, connection("https://gateway.test", None))
        .await
        .unwrap();
    assert!(
        replica
            .sign_in(&login_token(now, "litellm-lens-login", 60), "browser", now)
            .await
            .is_ok()
    );
}
