mod support;

use chrono::{DateTime, Duration, Utc};
use lens_auth::{
    Authentication, Error, ReadScope, SessionId, SessionRepository, Settings, StoreError,
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
#[tokio::test]
async fn sign_in_stores_a_hash_with_eight_hour_expiry(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    let view = authentication
        .sign_in(ADMIN, "new-session", now)
        .await
        .unwrap();
    assert_eq!(view.user_id, "lens-admin");
    assert_eq!(view.user_role, Role::ProxyAdmin);
    let values = authentication.sessions.values.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(
        values.get(SessionId::for_token("new-session").as_str()),
        Some(&(now + Duration::hours(8)))
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

#[rstest]
#[tokio::test]
async fn persisted_identity_keeps_restricted_scope_and_expiry(
    authentication: Authentication<Memory>,
    now: DateTime<Utc>,
) {
    let identity = lens_contract::auth::Identity {
        user_id: Some("google:subject".into()),
        user_role: lens_contract::auth::Role::InternalUserViewer,
        ..Default::default()
    };
    authentication
        .sessions
        .create_identity(
            &lens_auth::SessionId::for_token("sso-session"),
            now + chrono::Duration::minutes(1),
            &identity,
        )
        .await
        .unwrap();
    assert_eq!(
        authentication
            .authenticate(cookie("sso-session"), now)
            .await
            .unwrap(),
        identity
    );
    assert!(
        authentication
            .authenticate(cookie("sso-session"), now + chrono::Duration::minutes(1))
            .await
            .is_err()
    );
    authentication
        .sessions
        .revoke(&lens_auth::SessionId::for_token("sso-session"))
        .await
        .unwrap();
    assert!(
        authentication
            .authenticate(cookie("sso-session"), now)
            .await
            .is_err()
    );
}
