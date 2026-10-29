pub use funding_swap_error::FundingSwapError as StateError;

pub type StateResult<T> = core::result::Result<T, StateError>;
