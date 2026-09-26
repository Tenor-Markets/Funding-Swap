#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathError {
    Overflow,
    NegativeFee,
    InvalidDuration,
    InvalidTickStep,
}
