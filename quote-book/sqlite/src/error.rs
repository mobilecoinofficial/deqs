// Copyright (c) 2023 MobileCoin Inc.

use deqs_quote_book_api::Error as QuoteBookError;
use diesel::result::Error as DieselError;

/// Internal error type required to work around the fact that Diesel's
/// transaction code requires a From<DieselError> implementation.
/// This also saves a bunch of map_err boilerplate.
pub enum Error {
    Diesel(DieselError),
    QuoteBook(QuoteBookError),
    R2d2(r2d2::Error),
}

impl From<DieselError> for Error {
    fn from(e: DieselError) -> Self {
        Self::Diesel(e)
    }
}

impl From<QuoteBookError> for Error {
    fn from(e: QuoteBookError) -> Self {
        Self::QuoteBook(e)
    }
}

impl From<r2d2::Error> for Error {
    fn from(e: r2d2::Error) -> Self {
        Self::R2d2(e)
    }
}

impl From<Error> for QuoteBookError {
    fn from(e: Error) -> Self {
        match e {
            Error::Diesel(e) => Self::ImplementationSpecific(format!("diesel error: {e}")),
            Error::QuoteBook(e) => e,
            Error::R2d2(e) => Self::ImplementationSpecific(format!("r2d2 error: {e}")),
        }
    }
}
