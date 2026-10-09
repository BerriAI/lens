pub(super) fn default_bool<const V: bool>() -> bool {
    V
}
pub(super) fn default_nzu64<const V: u64>() -> std::num::NonZeroU64 {
    std::num::NonZeroU64::MIN.saturating_add(V - 1)
}
pub(super) fn job_stage() -> String {
    "Queued".to_owned()
}
pub(super) fn lens_settings_monthly_budget() -> f64 {
    100.0
}
pub(super) fn lens_settings_sample_percent() -> f64 {
    100.0
}
