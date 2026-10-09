#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("shorter than {0} characters")]
    TooShort(usize),
    #[error("longer than {0} characters")]
    TooLong(usize),
    #[error("invalid value")]
    InvalidValue,
}
