use thiserror::Error;

#[derive(Error, Debug)]
pub enum OptimizerError {
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Memory budget too small: {required} bytes required, {budget} available")]
    MemoryBudgetTooSmall { required: usize, budget: usize },

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Internal optimizer error: {0}")]
    InternalError(String),
}
