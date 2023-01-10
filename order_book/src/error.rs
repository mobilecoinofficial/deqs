// Copyright (c) 2023 MobileCoin Inc.

use displaydoc::Display;
use mc_transaction_extra::SignedContingentInputError;

/// Type for common order book errors
#[derive(Debug, Display, Eq, PartialEq)]
pub enum Error {
    /// SCI: {0}
    Sci(SignedContingentInputError),

    /// Unsupported SCI: {0}
    UnsupportedSci(String),

    /// Order already exists in bookl
    OrderAlreadyExists,

    /// Order not found
    OrderNotFound,

    /// Implementation specific error: {0}
    ImplementationSpecific(String),
}

impl From<SignedContingentInputError> for Error {
    fn from(err: SignedContingentInputError) -> Self {
        Self::Sci(err)
    }
}
