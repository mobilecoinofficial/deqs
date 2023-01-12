// Copyright (c) 2023 MobileCoin Inc.

use displaydoc::Display;
use mc_transaction_core::RevealedTxOutError;
use mc_transaction_extra::SignedContingentInputError;

/// Type for common order book errors
#[derive(Debug, Display, Eq, PartialEq)]
pub enum Error {
    /// SCI: {0}
    Sci(SignedContingentInputError),

    /// Unsupported SCI: {0}
    UnsupportedSci(String),

    /// Order already exists in book
    OrderAlreadyExists,

    /// Order not found
    OrderNotFound,

    /// Order cannot fulfill the desired amount ({0}) of base tokens
    InsufficientBaseTokens(u64),

    /// Implementation specific error: {0}
    ImplementationSpecific(String),

    /// RevealedTxOut: {0}
    RevealedTxOut(RevealedTxOutError),

    /// Time conversion error
    Time,
}

impl From<SignedContingentInputError> for Error {
    fn from(err: SignedContingentInputError) -> Self {
        Self::Sci(err)
    }
}

impl From<RevealedTxOutError> for Error {
    fn from(err: RevealedTxOutError) -> Self {
        Self::RevealedTxOut(err)
    }
}
