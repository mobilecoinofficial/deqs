// Copyright (c) 2023 MobileCoin Inc.

use deqs_quote_book_api::Error as QuoteBookError;
use displaydoc::Display;
use mc_transaction_extra::SignedContingentInputError;
use serde::{Deserialize, Serialize};

/// Errors returned over RPC calls.
/// Note that we have our own version of `QuoteBookError` here, because
/// it cannot derive serde traits (due to LedgerError not being
/// serde-compatible)
#[derive(Clone, Debug, Deserialize, Display, Eq, PartialEq, Serialize)]
pub enum RpcError {
    /// Quote Book: {0}
    QuoteBook(RpcQuoteBookError),

    /// Unexpected response
    UnexpectedResponse,

    /// Too many quotes requested
    TooManyQuotesRequested,
}

impl From<QuoteBookError> for RpcError {
    fn from(err: QuoteBookError) -> Self {
        Self::QuoteBook(err.into())
    }
}

#[derive(Clone, Debug, Deserialize, Display, Eq, PartialEq, Serialize)]
pub enum RpcQuoteBookError {
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

    /// Other errors
    Other(String),
}

impl From<QuoteBookError> for RpcQuoteBookError {
    fn from(err: QuoteBookError) -> Self {
        match err {
            QuoteBookError::Sci(err) => Self::Sci(err),
            QuoteBookError::UnsupportedSci(err) => Self::UnsupportedSci(err),
            QuoteBookError::QuoteAlreadyExists => Self::QuoteAlreadyExists,
            QuoteBookError::QuoteNotFound => Self::QuoteNotFound,
            QuoteBookError::QuoteIsStale => Self::QuoteIsStale,
            err => Self::Other(err.to_string()),
        }
    }
}
