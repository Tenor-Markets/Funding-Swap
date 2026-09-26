use anchor_lang::prelude::*;

#[error_code]
#[derive(Eq, PartialEq)]
pub enum StateError {
    ArithmeticOverflow,
    InvalidAuthority,
    InvalidClock,
    InvalidConfiguration,
    InvalidKey,
    InvalidReference,
    InvalidStatus,
    StaleObservation,
    InvalidTag,
    NotFound,
    CapacityExhausted,
    Conflict,
}

pub type StateResult<T> = core::result::Result<T, StateError>;
