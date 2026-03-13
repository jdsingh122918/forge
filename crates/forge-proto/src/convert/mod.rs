use thiserror::Error;

pub mod enums;
pub mod ids;
pub mod manifest;
pub mod run_graph;

pub use enums::{IntoProtoEnum, UnknownEnumValue};

/// Shared result type for proto/domain conversion helpers.
pub type Result<T> = std::result::Result<T, ConversionError>;

/// Local trait used for fallible conversions from generated proto messages.
pub trait TryFromProto<T>: Sized {
    fn try_from_proto(value: &T) -> Result<Self>;
}

/// Local trait used for encoding domain types into generated proto messages.
pub trait IntoProto<T> {
    fn into_proto(&self) -> T;
}

/// Errors raised while translating between generated proto messages and
/// `forge-common` domain types.
#[derive(Debug, Error)]
pub enum ConversionError {
    #[error(transparent)]
    UnknownEnumValue(#[from] UnknownEnumValue),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("negative value for {field}: {value}")]
    NegativeValue { field: &'static str, value: i64 },

    #[error("value out of range for {field}: {value}")]
    OutOfRange { field: &'static str, value: u64 },

    #[error("invalid memory limit `{0}`")]
    InvalidMemoryValue(String),

    #[error("invalid budget defaults: warn_at_percent must be <= 100, got {0}")]
    InvalidWarnThreshold(u8),

    #[error("unsupported budget field `{field}`: {reason}")]
    UnsupportedBudgetField {
        field: &'static str,
        reason: String,
    },
}

pub(crate) fn require_message<'a, T>(value: &'a Option<T>, field: &'static str) -> Result<&'a T> {
    value.as_ref().ok_or(ConversionError::MissingField(field))
}

pub(crate) fn non_negative_i64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| ConversionError::NegativeValue { field, value })
}

pub(crate) fn non_negative_i32(value: i32, field: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| ConversionError::NegativeValue {
        field,
        value: i64::from(value),
    })
}

pub(crate) fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| ConversionError::OutOfRange { field, value })
}

pub(crate) fn u32_to_i32(value: u32, field: &'static str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ConversionError::OutOfRange {
        field,
        value: u64::from(value),
    })
}
