// Copyright (c) 2023 MobileCoin Inc.

mod common;

use std::time::Duration;

use deqs_quote_book::{Error, InMemoryQuoteBook, Msg, QuoteBook, SynchronizedQuoteBook};
use mc_blockchain_types::BlockVersion;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_ledger_db::{Ledger, LedgerDB};
use postage::broadcast;
use rand::{rngs::StdRng, SeedableRng};

use mc_account_keys::AccountKey;
use mc_common::logger::{test_with_logger, Logger};
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_ledger_db::test_utils::{
    add_txos_and_key_images_to_ledger, add_txos_to_ledger, create_ledger, initialize_ledger,
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

#[test_with_logger]
fn basic_happy_flow(logger: Logger) {
    let ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);
    common::basic_happy_flow(&synchronized_quote_book);
}

#[test_with_logger]
fn cannot_add_invalid_sci(logger: Logger) {
    let ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);
    common::cannot_add_invalid_sci(&synchronized_quote_book);
}

#[test_with_logger]
fn get_quotes_filtering_works(logger: Logger) {
    let ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);
    common::get_quotes_filtering_works(&synchronized_quote_book);
}

#[test_with_logger]
fn get_quote_ids_works(logger: Logger) {
    let ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);
    common::get_quote_ids_works(&synchronized_quote_book);
}

#[test_with_logger]
fn get_quote_by_id_works(logger: Logger) {
    let ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);
    common::get_quote_by_id_works(&synchronized_quote_book);
}

#[test_with_logger]
fn cannot_add_sci_with_key_image_in_ledger(logger: Logger) {
    let mut ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);

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

    // Because the key image is already in the ledger, adding this sci should fail
    assert_eq!(
        synchronized_quote_book.add_sci(sci, None).unwrap_err(),
        Error::QuoteIsStale
    );

    // Adding a quote that isn't already in the ledger should work
    let sci = common::create_sci(&pair, 10, 20, &mut rng);
    let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone()]);
}

#[test_with_logger]
fn sci_that_are_added_to_ledger_are_removed_in_the_background(logger: Logger) {
    let mut ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);

    let pair = common::pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let sci_builder = common::create_sci_builder(&pair, 10, 20, &mut rng);
    let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
    let key_image = sci.key_image();
    // Adding a quote that isn't already in the ledger should work
    let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone()]);

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
        vec![key_image],
        &mut rng,
    )
    .unwrap();

    // Because the key image is already in the ledger, this sci should be removed in
    // the background after waiting for the quotebook to sync
    while synchronized_quote_book.get_current_block_index() < (ledger.num_blocks().unwrap() - 1) {
        std::thread::sleep(Duration::from_millis(1000));
    }
    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![]);
}

#[test_with_logger]
fn cannot_add_sci_past_tombstone_block(logger: Logger) {
    let pair = common::pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);

    // Because the tombstone block is lower than the number blocks already in the
    // ledger, adding this sci should fail
    let mut sci_builder = common::create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder.set_tombstone_block(ledger.num_blocks().unwrap() - 2);
    let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    assert_eq!(
        synchronized_quote_book.add_sci(sci, None).unwrap_err(),
        Error::QuoteIsStale
    );

    // Because the tombstone block is higher than the block index of the highest
    // block already in the ledger, adding this sci should pass
    let mut sci_builder2 = common::create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder2.set_tombstone_block(ledger.num_blocks().unwrap());
    let sci2 = sci_builder2.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    let quote2 = synchronized_quote_book.add_sci(sci2, None).unwrap();

    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote2.clone()]);

    // Because the tombstone block is 0, adding this sci should pass
    let mut sci_builder3 = common::create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder3.set_tombstone_block(0);
    let sci3 = sci_builder3.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    let quote3 = synchronized_quote_book.add_sci(sci3, None).unwrap();

    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote2.clone(), quote3.clone()]);

    // Because the tombstone block is equal to the current block index, adding this
    // sci should fail
    let mut sci_builder4 = common::create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder4.set_tombstone_block(ledger.num_blocks().unwrap() - 1);
    let sci4 = sci_builder4.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    assert_eq!(
        synchronized_quote_book.add_sci(sci4, None).unwrap_err(),
        Error::QuoteIsStale
    );
}

#[test_with_logger]
fn sci_past_tombstone_block_get_removed_in_the_background(logger: Logger) {
    let pair = common::pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let mut ledger = create_and_initialize_test_ledger();
    let (msg_bus_tx, _) = broadcast::channel::<Msg>(1000);

    let internal_quote_book = InMemoryQuoteBook::default();
    let starting_blocks = ledger.num_blocks().unwrap();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger.clone(), msg_bus_tx, logger);

    // Because the tombstone block is lower than the number blocks already in the
    // ledger, adding this sci should fail
    let mut sci_builder = common::create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder.set_tombstone_block(ledger.num_blocks().unwrap());
    let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

    assert_eq!(
        synchronized_quote_book.get_quotes(&pair, .., 0).unwrap(),
        vec![quote.clone()]
    );

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
    add_txos_to_ledger(&mut ledger, BlockVersion::MAX, &tx.prefix.outputs, &mut rng).unwrap();
    assert_eq!(ledger.num_blocks().unwrap(), starting_blocks + 1);
    // Because the ledger has advanced, this sci should be removed in the background
    // after waiting for the quotebook to sync
    while synchronized_quote_book.get_current_block_index() < (ledger.num_blocks().unwrap() - 1) {
        std::thread::sleep(Duration::from_millis(1000));
    }

    let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![]);
}
