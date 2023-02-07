// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_quote_book::{Error, InMemoryQuoteBook, QuoteBook, SynchronizedQuoteBook};
use mc_blockchain_types::BlockVersion;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_ledger_db::LedgerDB;
use rand::{rngs::StdRng, SeedableRng};

use mc_account_keys::AccountKey;
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_ledger_db::test_utils::{
    add_txos_and_key_images_to_ledger, create_ledger, initialize_ledger,
};
use mc_transaction_builder::test_utils::get_transaction;
use mc_transaction_core::TokenId;

fn create_and_initialize_test_ledger() -> LedgerDB {
    //Create a ledger_db
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let block_version = BlockVersion::MAX;
    let sender = AccountKey::random(&mut rng);
    let mut ledger = create_ledger();

    //Initialize that db
    let n_blocks = 3;
    initialize_ledger(block_version, &mut ledger, n_blocks, &sender, &mut rng);

    ledger
}

#[test]
fn basic_happy_flow() {
    let ledger = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
    common::basic_happy_flow(&synchronized_quote_book);
}

#[test]
fn cannot_add_invalid_sci() {
    let ledger = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
    common::cannot_add_invalid_sci(&synchronized_quote_book);
}

#[test]
fn get_quotes_filtering_works() {
    let ledger = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
    common::get_quotes_filtering_works(&synchronized_quote_book);
}

#[test]
fn get_quote_ids_works() {
    let ledger = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
    common::get_quote_ids_works(&synchronized_quote_book);
}

#[test]
fn get_quote_by_id_works() {
    let ledger = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
    common::get_quote_by_id_works(&synchronized_quote_book);
}

#[test]
fn cannot_add_stale_sci() {
    let mut ledger = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger.clone());

    let pair = common::pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let sci_builder = common::create_sci_builder(&pair, 10, 20, &mut rng);
    let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    let block_version = BlockVersion::MAX;
    let fog_resolver = MockFogResolver::default();

    let offerer_account = AccountKey::random(&mut rng);

    let tx = get_transaction(
        block_version,
        TokenId::from(0),
        2,
        2,
        &offerer_account,
        &offerer_account,
        fog_resolver,
        &mut rng,
    )
    .unwrap();
    add_txos_and_key_images_to_ledger(
        &mut ledger,
        BlockVersion::MAX,
        tx.prefix.outputs,
        vec![sci.key_image()],
        &mut rng,
    )
    .unwrap();

    //Because the key image is already in the ledger, adding this sci should fail
    assert_eq!(
        synchronized_quote_book.add_sci(sci, None).unwrap_err(),
        Error::QuoteIsStale
    );

    //Adding a quote that isn't already in the ledger should work
    let sci = common::create_sci(&pair, 10, 20, &mut rng);
    let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone()]);
}
