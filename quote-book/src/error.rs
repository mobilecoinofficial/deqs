// Copyright (c) 2023 MobileCoin Inc.

use displaydoc::Display;
use mc_ledger_db::Error as LedgerError;
use mc_transaction_core::RevealedTxOutError;
use mc_transaction_extra::SignedContingentInputError;
use serde::{Deserialize, Serialize};
use std::sync::PoisonError;

/// Type for common quote book errors
#[derive(Clone, Debug, Deserialize, Display, Eq, PartialEq, Serialize)]
pub enum Error {
    /// SCI: {0}
    Sci(SignedContingentInputError),

    /// Unsupported SCI: {0}
    UnsupportedSci(String),

    /// Quote already exists in book
    QuoteAlreadyExists,

    /// Quote not found
    QuoteNotFound,

    /// Quote has a spent keyimage
    QuoteIsStale,

    /// Ledger related error
    Ledger(LedgerError),

    /// Quote cannot fulfill the desired amount ({0}) of base tokens
    InsufficientBaseTokens(u64),

    /// Implementation specific error: {0}
    ImplementationSpecific(String),

    /// RevealedTxOut: {0}
    RevealedTxOut(RevealedTxOutError),

    /// LockPoisoned
    LockPoisoned,

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

impl From<LedgerError> for Error {
    fn from(err: LedgerError) -> Self {
        Self::Ledger(err)
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(_src: PoisonError<T>) -> Self {
        Error::LockPoisoned
    }
}
