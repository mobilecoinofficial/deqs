// Copyright (c) 2023 MobileCoin Inc.

use crate::deqs as api;
use deqs_quote_book::Error;

impl From<&Error> for api::QuoteStatusCode {
    fn from(src: &Error) -> Self {
        match src {
            Error::Sci(_) => Self::INVALID_SCI,
            Error::UnsupportedSci(_) => Self::UNSUPPORTED_SCI,
            Error::QuoteAlreadyExists => Self::QUOTE_ALREADY_EXISTS,
            _ => Self::OTHER,
        }
    }
}
