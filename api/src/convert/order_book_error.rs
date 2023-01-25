// Copyright (c) 2023 MobileCoin Inc.

use crate::deqs as api;
use deqs_order_book::Error;

impl From<&Error> for api::QuoteStatusCode {
    fn from(src: &Error) -> Self {
        match src {
            Error::Sci(_) => Self::INVALID_SCI,
            Error::UnsupportedSci(_) => Self::UNSUPPORTED_SCI,
            Error::OrderAlreadyExists => Self::ORDER_ALREADY_EXISTS,
            _ => Self::OTHER,
        }
    }
}
