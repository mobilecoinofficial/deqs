// Copyright (c) 2023 MobileCoin Inc.

// This module is reused by various tests, and Rust is annoying since each test
// file is compiled as an independent crate which makes Rust think methods in
// this file are unused if they are not called by each and every test file :/
#![allow(dead_code)]

use deqs_quote_book::{Error, Pair, QuoteBook};
use mc_account_keys::AccountKey;
use mc_blockchain_test_utils::{make_block_metadata, make_block_signature};
use mc_blockchain_types::{Block, BlockContents, BlockData, BlockVersion};
use mc_crypto_ring_signature::Error as RingSignatureError;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_ledger_db::Ledger;
use mc_transaction_builder::SignedContingentInputBuilder;
use mc_transaction_core::ring_signature::KeyImage;
use mc_transaction_extra::{SignedContingentInput, SignedContingentInputError};
use mc_transaction_types::{Amount, TokenId};
use rand::{rngs::StdRng, SeedableRng};
use rand_core::{CryptoRng, RngCore};

/// Default test pair
pub fn pair() -> Pair {
    Pair {
        base_token_id: TokenId::from(1),
        counter_token_id: TokenId::from(2),
    }
}

/// Create an SCI builder that offers some amount of a given token in exchange
/// for a different amount of another token. Returning the builder allows the
/// caller to customize the SCI further.
pub fn create_sci_builder(
    pair: &Pair,
    base_amount: u64,
    counter_amount: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInputBuilder<MockFogResolver> {
    deqs_mc_test_utils::create_sci_builder(
        pair.base_token_id,
        pair.counter_token_id,
        base_amount,
        counter_amount,
        rng,
    )
}

/// Create an SCI that offers some amount of a given token in exchange for a
/// different amount of another token. Returning the builder allows the caller
/// to customize the SCI further.
pub fn create_sci(
    pair: &Pair,
    base_amount: u64,
    counter_amount: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInput {
    deqs_mc_test_utils::create_sci(
        pair.base_token_id,
        pair.counter_token_id,
        base_amount,
        counter_amount,
        rng,
    )
}

/// Create a partial fill SCI that offers between required_base_change_amount
/// and base_amount_offered tokens, with a minimum required fill of
/// min_base_fill_amount.
pub fn create_partial_sci(
    pair: &Pair,
    base_amount_offered: u64,
    min_base_fill_amount: u64,
    required_base_change_amount: u64,
    counter_amount: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInput {
    deqs_mc_test_utils::create_partial_sci(
        pair.base_token_id,
        pair.counter_token_id,
        base_amount_offered,
        min_base_fill_amount,
        required_base_change_amount,
        counter_amount,
        rng,
    )
}

/// Test quote book basic happy flow
pub fn basic_happy_flow(quote_book: &impl QuoteBook) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Adding an quote should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let quote = quote_book.add_sci(sci, None).unwrap();

    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone()]);

    // Adding a second quote should work
    let sci = create_sci(&pair, 10, 200, &mut rng);
    let quote2 = quote_book.add_sci(sci, None).unwrap();

    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone(), quote2.clone()]);

    // Removing the quote by its id should work
    assert_eq!(quote, quote_book.remove_quote_by_id(quote.id()).unwrap());

    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote2.clone()]);

    // Can't remove the quote again
    assert_eq!(
        quote_book.remove_quote_by_id(quote.id()).unwrap_err(),
        Error::QuoteNotFound
    );
    assert_eq!(
        quote_book
            .remove_quotes_by_key_image(&quote.sci().key_image())
            .unwrap(),
        vec![],
    );

    // Removing quotes by key image should work
    assert_eq!(
        vec![quote2.clone()],
        quote_book
            .remove_quotes_by_key_image(&quote2.sci().key_image())
            .unwrap()
    );
    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![]);

    // Removing quotes by tombstone block should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let quote1 = quote_book.add_sci(sci, None).unwrap();
    let quote1_tombstone = quote
        .sci()
        .tx_in
        .input_rules
        .as_ref()
        .unwrap()
        .max_tombstone_block;

    let mut sci_builder = create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder.set_tombstone_block(quote1_tombstone - 1);
    let sci2 = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
    let quote2 = quote_book.add_sci(sci2, None).unwrap();

    assert_eq!(
        quote_book
            .remove_quotes_by_tombstone_block(quote1_tombstone - 1)
            .unwrap(),
        vec![quote2],
    );

    assert_eq!(
        quote_book
            .remove_quotes_by_tombstone_block(quote1_tombstone - 1)
            .unwrap(),
        vec![],
    );
    assert_eq!(
        quote_book
            .remove_quotes_by_tombstone_block(quote1_tombstone)
            .unwrap(),
        vec![quote1],
    );
    assert_eq!(
        quote_book
            .remove_quotes_by_tombstone_block(quote1_tombstone)
            .unwrap(),
        vec![],
    );
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
/// Test some invalid SCI scenarios
pub fn cannot_add_invalid_sci(quote_book: &impl QuoteBook) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Make an SCI invalid by adding some random required output
    let mut sci_builder = create_sci_builder(&pair, 10, 20, &mut rng);
    let recipient = AccountKey::random(&mut rng);
    sci_builder
        .add_required_output(
            Amount::new(10, TokenId::from(3)),
            &recipient.default_subaddress(),
            &mut rng,
        )
        .unwrap();

    let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

    assert_eq!(
        quote_book.add_sci(sci, None).unwrap_err(),
        Error::UnsupportedSci("Unsupported number of required/partial outputs 2/0".into())
    );

    // Make an SCI invalid by messing with the MLSAG
    let mut sci = create_sci(&pair, 10, 20, &mut rng);
    sci.mlsag.responses.pop();

    assert_eq!(
        quote_book.add_sci(sci, None).unwrap_err(),
        Error::Sci(SignedContingentInputError::RingSignature(
            RingSignatureError::LengthMismatch(22, 21),
        ))
    );
}

/// Test that get_quotes filter correctly.
pub fn get_quotes_filtering_works(quote_book: &impl QuoteBook) {
    let pair1 = pair();
    let pair2 = Pair {
        base_token_id: TokenId::from(10),
        counter_token_id: TokenId::from(2),
    };
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Offer for trading 100 pair1.base tokens into 1000 pair1.counter tokens
    let sci = create_sci(&pair1, 100, 1000, &mut rng);
    let p1_100_for_1000 = quote_book.add_sci(sci, None).unwrap();

    // Offer for partially trading up to 100 pair2.base tokens into 1000
    // pair2.counter tokens
    let sci = create_partial_sci(&pair2, 100, 1, 0, 1000, &mut rng);
    let p2_100_for_1000 = quote_book.add_sci(sci, None).unwrap();

    // Offer for partially trading 5 pair2.base tokens into 50 pair2.counter
    // tokens
    let sci = create_partial_sci(&pair2, 5, 1, 0, 50, &mut rng);
    let p2_5_for_50 = quote_book.add_sci(sci, None).unwrap();

    // Offer for partially trading 50 pair2.base tokens into 5 pair2.counter
    // tokens
    let sci = create_partial_sci(&pair2, 50, 1, 0, 5, &mut rng);
    let p2_50_for_5 = quote_book.add_sci(sci, None).unwrap();

    // Offer for exactly trading 50 pair2.base tokens into 3 pair2.counter
    // tokens
    let sci = create_sci(&pair2, 50, 3, &mut rng);
    let p2_50_for_3 = quote_book.add_sci(sci, None).unwrap();

    // Get all quotes at any quantity.
    let quotes = quote_book.get_quotes(&pair1, .., 0).unwrap();
    assert_eq!(quotes, vec![p1_100_for_1000.clone()]);

    let quotes = quote_book.get_quotes(&pair2, .., 0).unwrap();
    assert_eq!(
        quotes,
        vec![
            p2_50_for_3.clone(),     // rate is 16.6667
            p2_50_for_5.clone(),     // rate is 10
            p2_100_for_1000.clone(), // rate is 0.1
            p2_5_for_50.clone(),     // rate is 0.1
        ]
    );

    // Get all quotes but limit to the first 2
    let quotes = quote_book.get_quotes(&pair1, .., 2).unwrap();
    assert_eq!(quotes, vec![p1_100_for_1000.clone()]);

    let quotes = quote_book.get_quotes(&pair2, .., 2).unwrap();
    assert_eq!(quotes, vec![p2_50_for_3.clone(), p2_50_for_5.clone(),]);

    // Get all quotes that can provide an amount that is not available.
    let quotes = quote_book.get_quotes(&pair1, 10000.., 2).unwrap();
    assert_eq!(quotes, vec![]);

    let quotes = quote_book.get_quotes(&pair2, 10000.., 2).unwrap();
    assert_eq!(quotes, vec![]);

    // Get all quotes that can provide a subset of the amount requested.
    let quotes = quote_book.get_quotes(&pair2, 50.., 0).unwrap();
    assert_eq!(
        quotes,
        vec![
            p2_50_for_3.clone(),     // rate is 16.6667
            p2_50_for_5.clone(),     // rate is 10
            p2_100_for_1000.clone(), // rate is 0.1
        ]
    );

    let quotes = quote_book.get_quotes(&pair2, 51.., 0).unwrap();
    assert_eq!(
        quotes,
        vec![
            p2_100_for_1000.clone(), // rate is 0.1
        ]
    );

    let quotes = quote_book.get_quotes(&pair2, 50..70, 0).unwrap();
    assert_eq!(
        quotes,
        vec![
            p2_50_for_3.clone(),     // rate is 16.6667
            p2_50_for_5.clone(),     // rate is 10
            p2_100_for_1000.clone(), // rate is 0.1
        ]
    );
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
