// Copyright (c) 2023 MobileCoin Inc.

//! Convert to/from OrderId.

use crate::deqs as api;
use deqs_order_book::OrderId;
use mc_api::ConversionError;

/// Convert Rust OrderId to Protobuf OrderId.
impl From<&OrderId> for api::OrderId {
    fn from(src: &OrderId) -> Self {
        let mut order_id = api::OrderId::new();
        order_id.set_data((*src).to_vec());
        order_id
    }
}

/// Convert api::OrderId --> OrderId.
impl TryFrom<&api::OrderId> for OrderId {
    type Error = ConversionError;

    fn try_from(source: &api::OrderId) -> Result<Self, Self::Error> {
        let bytes: &[u8] = source.get_data();
        Ok(OrderId::try_from(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // OrderId --> api::OrderId
    fn test_order_id_from() {
        let source: OrderId = OrderId([4; 32]);
        let converted = api::OrderId::from(&source);
        assert_eq!(converted.data, source.to_vec());
    }

    #[test]
    // api::OrderId --> OrderId
    fn test_order_id_try_from() {
        let mut source = api::OrderId::new();
        source.set_data(OrderId([5; 32]).to_vec());

        // try_from should succeed.
        let order_id = OrderId::try_from(&source).unwrap();

        // order_id should have the correct value.
        assert_eq!(order_id, OrderId([5; 32]));
    }

    #[test]
    // `OrderId::try_from` should return ConversionError if the source contains the
    // wrong number of bytes.
    fn test_order_id_try_from_conversion_errors() {
        // Helper function asserts that a ConversionError::ArrayCastError is produced.
        fn expects_array_cast_error(bytes: &[u8]) {
            let mut source = api::OrderId::new();
            source.set_data(bytes.to_vec());
            match OrderId::try_from(&source).unwrap_err() {
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
