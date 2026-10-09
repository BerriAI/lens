use chrono::{TimeZone, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use lens_inference::{GatewayIdentity, GatewayPurpose};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use url::Url;

const SECRET: &str = "gateway-inference-fixture-key-32-bytes";

#[rstest]
#[case::short(31, false)]
#[case::minimum(32, true)]
#[case::maximum(512, true)]
#[case::long(513, false)]
fn signing_key_matches_gateway_length_limits(#[case] length: usize, #[case] valid: bool) {
    assert_eq!(
        GatewayIdentity::new(
            Url::parse("https://gateway.test").unwrap(),
            &"x".repeat(length)
        )
        .is_ok(),
        valid
    );
}

#[fixture]
fn identity() -> GatewayIdentity {
    GatewayIdentity::new(Url::parse("https://gateway.test/proxy").unwrap(), SECRET).unwrap()
}

#[rstest]
#[case::analysis(GatewayPurpose::Analysis, "analysis")]
#[case::signals(GatewayPurpose::Signals, "signals")]
fn signed_identity_has_bounded_purpose(
    identity: GatewayIdentity,
    #[case] purpose: GatewayPurpose,
    #[case] expected: &str,
) {
    let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
    let token = identity
        .token(
            &Url::parse("https://gateway.test/proxy/v1/chat/completions").unwrap(),
            purpose,
            now,
        )
        .unwrap()
        .unwrap();
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&["litellm-lens"]);
    validation.set_audience(&["litellm"]);
    validation.validate_exp = false;
    let claims = decode::<Value>(
        &token,
        &DecodingKey::from_secret(SECRET.as_bytes()),
        &validation,
    )
    .unwrap()
    .claims;
    assert_eq!(
        claims,
        json!({"iss":"litellm-lens","aud":"litellm","sub":"lens-internal","purpose":expected,"iat":now.timestamp(),"exp":now.timestamp()+30})
    );
    assert!(
        decode::<Value>(
            &token,
            &DecodingKey::from_secret(b"different-key"),
            &validation
        )
        .is_err()
    );
}

#[rstest]
#[case::different_host("https://provider.test/proxy/v1/chat/completions")]
#[case::different_port("https://gateway.test:8443/proxy/v1/chat/completions")]
#[case::different_scheme("http://gateway.test/proxy/v1/chat/completions")]
#[case::outside_path("https://gateway.test/v1/chat/completions")]
#[case::prefix_collision("https://gateway.test/proxy-other/v1/chat/completions")]
#[case::credentials("https://user:password@gateway.test/proxy/v1/chat/completions")]
#[case::username_only("https://user@gateway.test/proxy/v1/chat/completions")]
#[case::password_only("https://:password@gateway.test/proxy/v1/chat/completions")]
fn unrelated_endpoint_never_receives_identity(identity: GatewayIdentity, #[case] endpoint: &str) {
    assert!(
        identity
            .token(
                &Url::parse(endpoint).unwrap(),
                GatewayPurpose::Analysis,
                Utc::now()
            )
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case::short_secret("https://gateway.test", "short")]
#[case::scheme("ftp://gateway.test", SECRET)]
#[case::username("https://user@gateway.test", SECRET)]
#[case::password("https://user:password@gateway.test", SECRET)]
#[case::query("https://gateway.test?key=x", SECRET)]
#[case::fragment("https://gateway.test#section", SECRET)]
fn invalid_configuration_is_rejected(#[case] base: &str, #[case] secret: &str) {
    assert!(GatewayIdentity::new(Url::parse(base).unwrap(), secret).is_err());
}
