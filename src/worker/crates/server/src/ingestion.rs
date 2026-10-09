use std::{future::Future, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Method},
    routing::{delete, get},
};
use chrono::Utc;
use lens_auth::{
    Authentication, SessionRepository,
    ingestion::{Ingestion, IngestionRepository},
};
use lens_contract::ingestion::{IngestionKey, IngestionKeyCreated};
use serde_json::{Value, json};

use crate::{
    auth,
    datasets::validation,
    error::{IngestionHttpError, ValidationError},
};

pub trait CredentialPublisher: Send + Sync {
    fn refresh(&self) -> impl Future<Output = Result<bool, lens_auth::IngestionError>> + Send;
}

async fn create<R: SessionRepository, I: IngestionRepository, P: CredentialPublisher>(
    State(app): State<Arc<App<R, I, P>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<IngestionKeyCreated>, IngestionHttpError> {
    let parsed = validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request = validation::request(parsed, validation::Model::IngestionKey)?;
    let mut created = app.ingestion.create(&identity, request, Utc::now()).await?;
    created.active = app.publisher.refresh().await?;
    Ok(Json(created))
}

async fn list<R: SessionRepository, I: IngestionRepository, P: CredentialPublisher>(
    State(app): State<Arc<App<R, I, P>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Vec<IngestionKey>>, IngestionHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    Ok(Json(app.ingestion.list(&identity).await?))
}

async fn revoke<R: SessionRepository, I: IngestionRepository, P: CredentialPublisher>(
    State(app): State<Arc<App<R, I, P>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<bool>, IngestionHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    app.ingestion.revoke(&identity, &id).await?;
    app.publisher.refresh().await?;
    Ok(Json(true))
}

pub(crate) fn parse_expiry(
    value: &Value,
    path: &[Value],
) -> Result<chrono::DateTime<Utc>, ValidationError> {
    use speedate::{
        DateTime, DateTimeConfig, MicrosecondsPrecisionOverflowBehavior, Time, TimeConfig,
    };
    use strum::EnumMessage;
    let config = DateTimeConfig::builder()
        .time_config(
            TimeConfig::builder()
                .microseconds_precision_overflow_behavior(
                    MicrosecondsPrecisionOverflowBehavior::Truncate,
                )
                .unix_timestamp_offset(Some(0))
                .build(),
        )
        .build();
    let parsed = if let Some(text) = value.as_str() {
        DateTime::parse_str_with_config(text, &config).or_else(|_| {
            speedate::Date::parse_str(text).map(|date| DateTime {
                date,
                time: Time {
                    hour: 0,
                    minute: 0,
                    second: 0,
                    microsecond: 0,
                    tz_offset: None,
                },
            })
        })
    } else if let Some(number) = value.as_i64() {
        DateTime::from_timestamp_with_config(number, 0, &config)
    } else if let Some(number) = value.as_f64() {
        DateTime::from_float_with_config(number, &config)
    } else {
        return Err(validation::failure(
            "datetime_type",
            path,
            "Input should be a valid datetime",
            value.clone(),
            None,
        ));
    };
    let parsed = parsed.map_err(|error| {
        let reason = error.get_documentation().unwrap_or("invalid datetime");
        let (kind, prefix) = if value.is_string() {
            (
                "datetime_from_date_parsing",
                "Input should be a valid datetime or date",
            )
        } else {
            ("datetime_parsing", "Input should be a valid datetime")
        };
        validation::failure(
            kind,
            path,
            format!("{prefix}, {reason}"),
            value.clone(),
            Some(json!({"error":reason})),
        )
    })?;
    if parsed.time.tz_offset.is_none() {
        return Err(validation::failure(
            "timezone_aware",
            path,
            "Input should have timezone info",
            value.clone(),
            None,
        ));
    }
    Ok(
        chrono::DateTime::from_timestamp(parsed.timestamp_tz(), parsed.time.microsecond * 1000)
            .unwrap(),
    )
}

struct App<R, I, P> {
    authentication: Arc<Authentication<R>>,
    ingestion: Ingestion<I>,
    publisher: Arc<P>,
}

pub fn router<R, I, P>(
    authentication: Arc<Authentication<R>>,
    ingestion: Ingestion<I>,
    publisher: Arc<P>,
) -> Router
where
    R: SessionRepository + 'static,
    I: IngestionRepository + 'static,
    P: CredentialPublisher + 'static,
{
    Router::new()
        .route(
            "/lens/tracing/keys",
            get(list::<R, I, P>).post(create::<R, I, P>),
        )
        .route("/lens/tracing/keys/{key_id}", delete(revoke::<R, I, P>))
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn(
            crate::tracing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            ingestion,
            publisher,
        }))
}

#[cfg(test)]
mod tests {
    use super::parse_expiry;
    use rstest::rstest;
    use serde_json::{Value, json};

    #[rstest]
    #[case::rfc3339(json!("2030-01-01T01:00:00+01:00"), "2030-01-01T00:00:00Z")]
    #[case::seconds(json!(1893456000), "2030-01-01T00:00:00Z")]
    #[case::milliseconds(json!(1893456000000_i64), "2030-01-01T00:00:00Z")]
    #[case::numeric_string(json!("1893456000000"), "2030-01-01T00:00:00Z")]
    #[case::fractional_seconds(json!(-0.5), "1969-12-31T23:59:59.500Z")]
    #[case::truncate_precision(json!("2030-01-01T00:00:00.123456789Z"), "2030-01-01T00:00:00.123456Z")]
    fn expiry_accepts_existing_datetime_and_epoch_forms(
        #[case] input: Value,
        #[case] expected: &str,
    ) {
        assert_eq!(
            parse_expiry(&input, &[]).unwrap(),
            chrono::DateTime::parse_from_rfc3339(expected).unwrap()
        );
    }

    #[rstest]
    #[case::date(json!("2030-01-01"), "timezone_aware", "Input should have timezone info")]
    #[case::datetime(json!("2030-01-01T00:00:00"), "timezone_aware", "Input should have timezone info")]
    #[case::boolean(json!(true), "datetime_type", "Input should be a valid datetime")]
    #[case::bad_month(json!("2030-13-01"), "datetime_from_date_parsing", "Input should be a valid datetime or date, month value is outside expected range of 1-12")]
    fn expiry_validation_preserves_pydantic_failure_details(
        #[case] input: Value,
        #[case] kind: &str,
        #[case] message: &str,
    ) {
        let error = parse_expiry(&input, &[json!("body"), json!("expires_at")]).unwrap_err();
        assert_eq!(error["type"], kind);
        assert_eq!(error["msg"], message);
        assert_eq!(error["input"], input);
        assert_eq!(error["loc"], json!(["body", "expires_at"]));
    }
}
