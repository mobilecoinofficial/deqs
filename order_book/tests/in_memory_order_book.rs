// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_order_book::InMemoryOrderBook;

#[test]
fn basic_happy_flow() {
    let order_book = InMemoryOrderBook::new();
    common::basic_happy_flow(&order_book);
}

#[test]
fn cannot_add_invalid_sci() {
    let order_book = InMemoryOrderBook::new();
    common::cannot_add_invalid_sci(&order_book);
}
