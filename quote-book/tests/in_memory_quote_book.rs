// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_quote_book::InMemoryQuoteBook;
use mc_ledger_db::test_utils::MockLedger;

#[test]
fn basic_happy_flow() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger);
    common::basic_happy_flow(&quote_book);
}

#[test]
fn cannot_add_invalid_sci() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger);
    common::cannot_add_invalid_sci(&quote_book);
}

#[test]
fn get_quotes_filtering_works() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger);
    common::get_quotes_filtering_works(&quote_book);
}
