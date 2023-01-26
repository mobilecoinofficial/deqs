// Copyright (c) 2023 MobileCoin Inc.

//! Convert to/from QuoteId.

use crate::deqs as api;
use deqs_quote_book::QuoteId;
use mc_api::ConversionError;

/// Convert Rust QuoteId to Protobuf QuoteId.
impl From<&QuoteId> for api::QuoteId {
    fn from(src: &QuoteId) -> Self {
        let mut quote_id = api::QuoteId::new();
        quote_id.set_data((*src).to_vec());
        quote_id
    }
}

/// Convert api::QuoteId --> QuoteId.
impl TryFrom<&api::QuoteId> for QuoteId {
    type Error = ConversionError;

    fn try_from(source: &api::QuoteId) -> Result<Self, Self::Error> {
        let bytes: &[u8] = source.get_data();
        Ok(QuoteId::try_from(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // QuoteId --> api::QuoteId
    fn test_quote_id_from() {
        let source: QuoteId = QuoteId([4; 32]);
        let converted = api::QuoteId::from(&source);
        assert_eq!(converted.data, source.to_vec());
    }

    #[test]
    // api::QuoteId --> QuoteId
    fn test_quote_id_try_from() {
        let mut source = api::QuoteId::new();
        source.set_data(QuoteId([5; 32]).to_vec());

        // try_from should succeed.
        let quote_id = QuoteId::try_from(&source).unwrap();

        // quote_id should have the correct value.
        assert_eq!(quote_id, QuoteId([5; 32]));
    }

    #[test]
    // `QuoteId::try_from` should return ConversionError if the source contains the
    // wrong number of bytes.
    fn test_quote_id_try_from_conversion_errors() {
        // Helper function asserts that a ConversionError::ArrayCastError is produced.
        fn expects_array_cast_error(bytes: &[u8]) {
            let mut source = api::QuoteId::new();
            source.set_data(bytes.to_vec());
            match QuoteId::try_from(&source).unwrap_err() {
                ConversionError::ArrayCastError => {} // Expected outcome.
                _ => panic!(),
            }
        }

        // Too many bytes should produce an ArrayCastError.
        expects_array_cast_error(&[11u8; 119]);

        // Too few bytes should produce an ArrayCastError.
        expects_array_cast_error(&[11u8; 3]);
    }
}
