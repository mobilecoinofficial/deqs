// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_quote_book::InMemoryQuoteBook;

#[test]
fn basic_happy_flow() {
    let quote_book = InMemoryQuoteBook::default();
    common::basic_happy_flow(&quote_book);
}

#[test]
fn cannot_add_invalid_sci() {
    let quote_book = InMemoryQuoteBook::default();
    common::cannot_add_invalid_sci(&quote_book);
}

#[test]
fn get_quotes_filtering_works() {
    let quote_book = InMemoryQuoteBook::default();
    common::get_quotes_filtering_works(&quote_book);
}
