pub use fixed_perp_error::FixedPerpError as StateError;

pub type StateResult<T> = core::result::Result<T, StateError>;
