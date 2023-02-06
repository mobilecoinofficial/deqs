// Copyright (c) 2023 MobileCoin Inc.

mod common;

use deqs_quote_book::{InMemoryQuoteBook, SynchronizedQuoteBook};
use mc_ledger_db::test_utils::MockLedger;

#[test]
fn basic_happy_flow() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(quote_book, ledger);
    common::basic_happy_flow(&synchronized_quote_book);
}

#[test]
fn cannot_add_invalid_sci() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(quote_book, ledger);
    common::cannot_add_invalid_sci(&synchronized_quote_book);
}

#[test]
fn get_quotes_filtering_works() {
    let ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(quote_book, ledger);
    common::get_quotes_filtering_works(&synchronized_quote_book);
}

#[test]
fn cannot_add_stale_sci() {
    let mut ledger = MockLedger::default();
    let quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(quote_book, ledger.clone());
    common::add_quote_already_in_ledger_should_fail(&synchronized_quote_book, &mut ledger);
}

/// Test adding a quote which is already in the ledger
pub fn add_quote_already_in_ledger_should_fail(
    quote_book: &impl QuoteBook,
    ledger: &mut impl Ledger,
) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let sci_builder = create_sci_builder(&pair, 10, 20, &mut rng);
    let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
    add_key_image_to_ledger(ledger, BlockVersion::MAX, vec![sci.key_image()], &mut rng).unwrap();

    //Because the key image is already in the ledger, adding this sci should fail
    assert_eq!(
        quote_book.add_sci(sci, None).unwrap_err(),
        Error::QuoteIsStale
    );

    //Adding a quote that isn't already in the ledger should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let quote = quote_book.add_sci(sci, None).unwrap();

    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone()]);
}

/// Adds a block containing the given keyimage to the ledger and returns the new
/// block.
///
/// # Arguments
/// # TODO(wjuan): This should be refactored in along with the test_utils for adding to ledger in the mobilecoin repo.
/// * `ledger` - Ledger instance.
/// * `block_version` - The block version to use.
/// * `key image` - The key-image to be added
/// * `rng` - Random number generator.
pub fn add_key_image_to_ledger(
    ledger_db: &mut impl Ledger,
    block_version: BlockVersion,
    key_images: Vec<KeyImage>,
    rng: &mut (impl CryptoRng + RngCore),
) -> Result<BlockData, Error> {
    let num_blocks = ledger_db.num_blocks()?;
    let block_contents = BlockContents {
        key_images,
        ..Default::default()
    };
    let new_block = if num_blocks > 0 {
        let parent = ledger_db.get_block(num_blocks - 1)?;

        Block::new_with_parent(block_version, &parent, &Default::default(), &block_contents)
    } else {
        Block::new_origin_block(&block_contents.outputs)
    };

    let signature = make_block_signature(&new_block, rng);
    let metadata = make_block_metadata(new_block.id.clone(), rng);
    let block_data = BlockData::new(new_block, block_contents, signature, metadata);

    ledger_db.append_block_data(&block_data)?;

    Ok(block_data)
}