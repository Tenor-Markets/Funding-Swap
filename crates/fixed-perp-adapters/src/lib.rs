#![no_std]

pub mod error;
pub mod jupiter;
pub mod phoenix;

pub use error::AdapterError;

use core::convert::TryFrom;
use solana_pubkey::Pubkey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountView<'a> {
    pub key: Pubkey,
    pub owner: Pubkey,
    pub data: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationLimits {
    pub max_source_delta: u128,
    pub max_source_age_seconds: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationContext {
    pub previous_source_checkpoint: u128,
    pub previous_protocol_checkpoint_at: i64,
    pub protocol_now: i64,
}

pub(crate) fn validate_observation_timestamps(
    context: ObservationContext,
    source_last_update: i64,
    limits: ObservationLimits,
) -> Result<(), AdapterError> {
    if context.protocol_now < 0 {
        return Err(AdapterError::InvalidClock);
    }
    if context.previous_protocol_checkpoint_at < 0
        || context.previous_protocol_checkpoint_at > context.protocol_now
    {
        return Err(AdapterError::InvalidProtocolCheckpointTimestamp);
    }
    if source_last_update < 0 || source_last_update > context.protocol_now {
        return Err(AdapterError::InvalidSourceAnchor);
    }

    let source_age = u64::try_from(
        context
            .protocol_now
            .checked_sub(source_last_update)
            .ok_or(AdapterError::ArithmeticOverflow)?,
    )
    .map_err(|_| AdapterError::ArithmeticOverflow)?;
    if source_age > limits.max_source_age_seconds {
        return Err(AdapterError::SourceTooOld);
    }

    Ok(())
}
