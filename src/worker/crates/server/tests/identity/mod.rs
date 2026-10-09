pub fn delegated(identity: lens_contract::auth::Identity) -> String {
    let now = chrono::Utc::now().timestamp();
    let subject = identity
        .user_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .or(identity.token.as_deref())
        .unwrap();
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &serde_json::json!({"iss":"litellm","aud":"litellm-lens","sub":subject,"iat":now,"exp":now+60,"identity":identity}),
        &jsonwebtoken::EncodingKey::from_secret(super::support::SECRET.as_bytes()),
    ).unwrap()
}
