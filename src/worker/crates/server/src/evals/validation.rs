use crate::error::EvalError;
use lens_contract::eval::{CreateEvalRun, scorer_names};
use std::sync::LazyLock;

static CREATE: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    jsonschema::validator_for(&serde_json::to_value(schemars::schema_for!(CreateEvalRun)).unwrap())
        .unwrap()
});

pub(super) fn create(body: &[u8]) -> Result<CreateEvalRun, EvalError> {
    let value = serde_json::from_slice(body).map_err(EvalError::InvalidRequest)?;
    if !CREATE.is_valid(&value) {
        return Err(EvalError::InvalidSpec);
    }
    let spec: CreateEvalRun = serde_json::from_value(value).map_err(EvalError::InvalidRequest)?;
    let names = scorer_names(&spec.scorers);
    if spec.gate.min.iter().any(|(name, minimum)| {
        !names.contains(name) || !minimum.is_finite() || !(0.0..=1.0).contains(minimum)
    }) {
        return Err(EvalError::InvalidSpec);
    }
    let timeout = chrono::TimeDelta::try_milliseconds(
        i64::try_from(spec.timeout_per_trial_ms).map_err(|_| EvalError::InvalidSpec)?,
    )
    .ok_or(EvalError::InvalidSpec)?;
    chrono::Utc::now()
        .checked_add_signed(timeout)
        .ok_or(EvalError::InvalidSpec)?;
    Ok(spec)
}
