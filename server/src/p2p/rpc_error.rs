// Copyright (c) 2023 MobileCoin Inc.

use displaydoc::Display;
use mc_transaction_extra::SignedContingentInputError;
use serde::{Deserialize, Serialize};

/// Errors returned over RPC calls.
/// Note that we have our own version of `deqs_quote_book::Error` here, because
/// it cannot derive serde traits (due to LedgerError not being
/// serde-compatible)
#[derive(Clone, Debug, Deserialize, Display, Eq, PartialEq, Serialize)]
pub enum RpcError {
    /// Quote Book: {0}
    QuoteBook(RpcQuoteBookError),

    /// Unexpected response
    UnexpectedResponse,
}

impl From<deqs_quote_book::Error> for RpcError {
    fn from(err: deqs_quote_book::Error) -> Self {
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

impl From<deqs_quote_book::Error> for RpcQuoteBookError {
    fn from(err: deqs_quote_book::Error) -> Self {
        match err {
            deqs_quote_book::Error::Sci(err) => Self::Sci(err),
            deqs_quote_book::Error::UnsupportedSci(err) => Self::UnsupportedSci(err),
            deqs_quote_book::Error::QuoteAlreadyExists => Self::QuoteAlreadyExists,
            deqs_quote_book::Error::QuoteNotFound => Self::QuoteNotFound,
            deqs_quote_book::Error::QuoteIsStale => Self::QuoteIsStale,
            err => Self::Other(err.to_string()),
        }
    }
}
