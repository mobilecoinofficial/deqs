// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_quote_book::{InMemoryQuoteBook, SynchronizedQuoteBook};
use mc_ledger_db::test_utils::MockLedger;

#[test]
fn basic_happy_flow() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger);
    let synchronized_quote_book = SynchronizedQuoteBook(quote_book, ledger);
    common::basic_happy_flow(&synchronized_quote_book);
}

#[test]
fn cannot_add_invalid_sci() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger);
    let synchronized_quote_book = SynchronizedQuoteBook(quote_book, ledger);
    common::cannot_add_invalid_sci(&synchronized_quote_book);
}

#[test]
fn get_quotes_filtering_works() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger);
    let synchronized_quote_book = SynchronizedQuoteBook(quote_book, ledger);
    common::get_quotes_filtering_works(&synchronized_quote_book);
}

#[test]
fn cannot_add_stale_sci() {
    let mut ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::new(ledger.clone());
    let synchronized_quote_book = SynchronizedQuoteBook(quote_book, ledger);
    common::add_quote_already_in_ledger_should_fail(&synchronized_quote_book, &mut ledger);
}
