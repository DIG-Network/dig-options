//! The crate error taxonomy.
//!
//! Every fallible operation in dig-options returns [`Result`], whose error is [`Error`].
//! The variants separate the three failure sources a pure builder can hit: a lower-level
//! driver failure while constructing a spend, a signer failure while computing the required
//! signatures, and caller-supplied input that cannot produce a valid spend.

use chia_wallet_sdk::driver::DriverError;
use chia_wallet_sdk::signer::SignerError;

/// The result of a dig-options operation.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while building an option coin spend or reporting the
/// signatures a coin spend requires.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A failure in the underlying chia-wallet-sdk driver while constructing a spend
    /// (allocation, currying, puzzle assembly, launcher mint).
    #[error("driver error: {0}")]
    Driver(#[from] DriverError),

    /// A failure while computing the BLS signatures a coin spend requires.
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),

    /// Caller-supplied input that cannot produce a valid spend (e.g. a zero underlying
    /// amount, an underfunded coin, a wrong-party clawback, or an unsupported strike type).
    /// The message states the precise violation.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl Error {
    /// Construct an [`Error::InvalidInput`] from any displayable message.
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Error::InvalidInput(message.into())
    }
}
