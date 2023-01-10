// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_order_book::{InMemoryOrderBook, InMemoryOrderBookError};
use mc_crypto_ring_signature::Error as RingSignatureError;

#[test]
fn basic_happy_flow() {
    let order_book = InMemoryOrderBook::new();
    common::basic_happy_flow(&order_book, InMemoryOrderBookError::SciNotFound);
}

#[test]
fn cannot_add_invalid_sci() {
    let order_book = InMemoryOrderBook::new();
    common::cannot_add_invalid_sci(
        &order_book,
        InMemoryOrderBookError::IncorrectNumberOfOutputs,
        InMemoryOrderBookError::Sci(
            mc_transaction_extra::SignedContingentInputError::RingSignature(
                RingSignatureError::LengthMismatch(22, 21),
            ),
        ),
    );
}
