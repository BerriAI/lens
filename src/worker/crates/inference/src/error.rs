#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Gateway inference requires a valid HTTP URL and a signing secret of 32 to 512 bytes")]
    GatewayConfiguration,
    #[error("Gateway inference identity could not be signed")]
    GatewayIdentity(#[from] jsonwebtoken::errors::Error),
    #[error("Monthly lens budget reached; increase it or wait for next month")]
    MonthlyBudget,
    #[error(
        "This model request needs up to ${amount:.3}, but ${available:.3} remains in the investigation budget. Use a smaller deployment output allowance or increase the limit."
    )]
    RequestBudget { amount: f64, available: f64 },
    #[error("Job was cancelled or reassigned")]
    JobReassigned,
    #[error("Analysis budget reservation expired; retry the investigation")]
    ReservationExpired,
    #[error(
        "Pricing is not configured for {model}. Set input_cost_per_token and output_cost_per_token on its deployment before running an investigation."
    )]
    Pricing { model: String },
    #[error(
        "Output capacity is unknown for {model}. Set model_info.max_output_tokens to the model's supported output capacity or configure max_tokens on its deployment."
    )]
    OutputCapacity { model: String },
    #[error("Analysis model is no longer available")]
    ModelUnavailable,
    #[error("Malformed legacy Lens prompt; send structured messages.")]
    MalformedPrompt,
    #[error(transparent)]
    Conversion(#[from] lens_contract::ConversionError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
