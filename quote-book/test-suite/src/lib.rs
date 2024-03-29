// Copyright (c) 2023 MobileCoin Inc.

#![feature(assert_matches)]

use deqs_quote_book_api::{Error, Pair, Quote, QuoteBook, QuoteId};
use mc_account_keys::AccountKey;
use mc_crypto_ring_signature::Error as RingSignatureError;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_ledger_db::LedgerDB;
use mc_transaction_builder::SignedContingentInputBuilder;
use mc_transaction_extra::{SignedContingentInput, SignedContingentInputError};
use mc_transaction_types::{Amount, TokenId};
use rand::{rngs::StdRng, SeedableRng};
use rand_core::{CryptoRng, RngCore};
use std::{assert_matches::assert_matches, collections::HashSet};

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
    ledger_db: Option<&LedgerDB>,
) -> SignedContingentInputBuilder<MockFogResolver> {
    deqs_mc_test_utils::create_sci_builder(
        pair.base_token_id,
        pair.counter_token_id,
        base_amount,
        counter_amount,
        rng,
        ledger_db,
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
    ledger_db: Option<&LedgerDB>,
) -> SignedContingentInput {
    deqs_mc_test_utils::create_sci(
        pair.base_token_id,
        pair.counter_token_id,
        base_amount,
        counter_amount,
        rng,
        ledger_db,
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
    ledger_db: Option<&LedgerDB>,
) -> SignedContingentInput {
    deqs_mc_test_utils::create_partial_sci(
        pair.base_token_id,
        pair.counter_token_id,
        base_amount_offered,
        min_base_fill_amount,
        required_base_change_amount,
        counter_amount,
        rng,
        ledger_db,
    )
}

/// Test quote book basic happy flow
pub fn basic_happy_flow(quote_book: &impl QuoteBook, ledger_db: Option<&LedgerDB>) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Adding an quote should work
    let sci = create_sci(&pair, 10, 20, &mut rng, ledger_db);
    let quote = quote_book.add_sci(sci, None).unwrap();

    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote.clone()]);

    // Adding a second quote should work
    let sci = create_sci(&pair, 10, 200, &mut rng, ledger_db);
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

    // Removing quotes by tombstone block should work. Quotes with a zero tombstone
    // should survive.
    let sci = create_sci(&pair, 10, 20, &mut rng, ledger_db);
    let quote1 = quote_book.add_sci(sci, None).unwrap();
    let quote1_tombstone = quote
        .sci()
        .tx_in
        .input_rules
        .as_ref()
        .unwrap()
        .max_tombstone_block;

    let mut sci_builder = create_sci_builder(&pair, 10, 20, &mut rng, ledger_db);
    sci_builder.set_tombstone_block(quote1_tombstone - 1);
    let sci2 = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
    let quote2 = quote_book.add_sci(sci2, None).unwrap();

    let mut sci_builder = create_sci_builder(&pair, 10, 20, &mut rng, ledger_db);
    sci_builder.set_tombstone_block(0);
    let sci3 = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
    let quote3 = quote_book.add_sci(sci3, None).unwrap();

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

    // The quote with 0 max tombstone should still be around
    let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
    assert_eq!(quotes, vec![quote3]);
}

/// Test some invalid SCI scenarios
pub fn cannot_add_invalid_sci(quote_book: &impl QuoteBook, ledger_db: Option<&LedgerDB>) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Make an SCI invalid by adding some random required output
    let mut sci_builder = create_sci_builder(&pair, 10, 20, &mut rng, ledger_db);
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
    let mut sci = create_sci(&pair, 10, 20, &mut rng, ledger_db);
    sci.mlsag.responses.pop();

    assert_eq!(
        quote_book.add_sci(sci, None).unwrap_err(),
        Error::Sci(SignedContingentInputError::RingSignature(
            RingSignatureError::LengthMismatch(22, 21),
        ))
    );
}

pub fn cannot_add_duplicate_sci(quote_book: &impl QuoteBook, ledger_db: Option<&LedgerDB>) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let sci = create_sci(&pair, 10, 20, &mut rng, ledger_db);
    let quote1 = quote_book.add_sci(sci.clone(), None).unwrap();

    // Trying to add the exact same SCI should fail
    assert_matches!(
        quote_book.add_sci(sci.clone(), None).unwrap_err(),
        Error::QuoteAlreadyExists { existing_quote } if *existing_quote == quote1
    );

    // Trying to add a different SCI with the same key image should fail
    let mut sci2 = sci.clone();
    sci2.tx_out_global_indices[0] += 1;

    assert_matches!(
        quote_book.add_sci(sci2.clone(), None).unwrap_err(),
        Error::QuoteAlreadyExists { existing_quote } if *existing_quote == quote1
    );

    // Test sanity: Quote id should be different but key image should be
    // identical.
    assert_eq!(sci.key_image(), sci2.key_image());

    let quote2 = Quote::new(sci2, None).unwrap();
    assert_ne!(quote1.id(), quote2.id());
    assert_eq!(quote1.key_image(), quote2.key_image());
}

/// Test that get_quotes filter correctly.
pub fn get_quotes_filtering_works(quote_book: &impl QuoteBook, ledger_db: Option<&LedgerDB>) {
    let pair1 = pair();
    let pair2 = Pair {
        base_token_id: TokenId::from(10),
        counter_token_id: TokenId::from(2),
    };
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Offer for trading 100 pair1.base tokens into 1000 pair1.counter tokens
    let sci = create_sci(&pair1, 100, 1000, &mut rng, ledger_db);
    let p1_100_for_1000 = quote_book.add_sci(sci, None).unwrap();

    // Offer for partially trading up to 100 pair2.base tokens into 1000
    // pair2.counter tokens
    let sci = create_partial_sci(&pair2, 100, 1, 0, 1000, &mut rng, ledger_db);
    let p2_100_for_1000 = quote_book.add_sci(sci, None).unwrap();

    // Offer for partially trading 5 pair2.base tokens into 50 pair2.counter
    // tokens
    let sci = create_partial_sci(&pair2, 5, 1, 0, 50, &mut rng, ledger_db);
    let p2_5_for_50 = quote_book.add_sci(sci, None).unwrap();

    // Offer for partially trading 50 pair2.base tokens into 5 pair2.counter
    // tokens
    let sci = create_partial_sci(&pair2, 50, 1, 0, 5, &mut rng, ledger_db);
    let p2_50_for_5 = quote_book.add_sci(sci, None).unwrap();

    // Offer for exactly trading 50 pair2.base tokens into 3 pair2.counter
    // tokens
    let sci = create_sci(&pair2, 50, 3, &mut rng, ledger_db);
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
            p2_5_for_50,             // rate is 0.1
        ]
    );

    // Get all quotes but limit to the first 2
    let quotes = quote_book.get_quotes(&pair1, .., 2).unwrap();
    assert_eq!(quotes, vec![p1_100_for_1000]);

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
            p2_50_for_3,     // rate is 16.6667
            p2_50_for_5,     // rate is 10
            p2_100_for_1000, // rate is 0.1
        ]
    );
}

/// Test that get_quote_ids works.
pub fn get_quote_ids_works(quote_book: &impl QuoteBook, ledger_db: Option<&LedgerDB>) {
    let pair1 = pair();
    let pair2 = Pair {
        base_token_id: TokenId::from(10),
        counter_token_id: TokenId::from(2),
    };
    let pair3 = Pair {
        base_token_id: TokenId::from(20),
        counter_token_id: TokenId::from(2),
    };
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let pair1_sci1 = create_sci(&pair1, 100, 1000, &mut rng, ledger_db);
    let quote1 = quote_book.add_sci(pair1_sci1, None).unwrap();

    let pair1_sci2 = create_sci(&pair1, 100, 1000, &mut rng, ledger_db);
    let quote2 = quote_book.add_sci(pair1_sci2, None).unwrap();

    let pair2_sci3 = create_sci(&pair2, 100, 1000, &mut rng, ledger_db);
    let quote3 = quote_book.add_sci(pair2_sci3, None).unwrap();

    // Without filtering, we should get all quote ids.
    let quote_ids = quote_book.get_quote_ids(None).unwrap();
    assert_eq!(
        HashSet::from_iter(vec![quote1.id(), quote2.id(), quote3.id()]),
        quote_ids.iter().collect::<HashSet<&QuoteId>>(),
    );

    // Filter to a specific pair
    let quote_ids = quote_book.get_quote_ids(Some(&pair1)).unwrap();
    assert_eq!(
        HashSet::from_iter(vec![quote1.id(), quote2.id()]),
        quote_ids.iter().collect::<HashSet<&QuoteId>>(),
    );

    let quote_ids = quote_book.get_quote_ids(Some(&pair2)).unwrap();
    assert_eq!(
        HashSet::from_iter(vec![quote3.id(), quote3.id()]),
        quote_ids.iter().collect::<HashSet<&QuoteId>>(),
    );

    let quote_ids = quote_book.get_quote_ids(Some(&pair3)).unwrap();
    assert_eq!(quote_ids, vec![]);
}

/// Test that get_quote_by_id works
pub fn get_quote_by_id_works(quote_book: &impl QuoteBook, ledger_db: Option<&LedgerDB>) {
    let pair1 = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let sci1 = create_sci(&pair1, 100, 1000, &mut rng, ledger_db);
    let quote1 = quote_book.add_sci(sci1, None).unwrap();

    let sci2 = create_sci(&pair1, 100, 1000, &mut rng, ledger_db);
    let quote2 = Quote::new(sci2, None).unwrap();

    assert_eq!(
        quote_book.get_quote_by_id(quote1.id()).unwrap(),
        Some(quote1.clone())
    );

    assert_eq!(quote_book.get_quote_by_id(quote2.id()).unwrap(), None);
}
