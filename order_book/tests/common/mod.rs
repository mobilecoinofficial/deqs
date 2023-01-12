// Copyright (c) 2023 MobileCoin Inc.

// This module is reused by various tests, and Rust is annoying since each test
// file is compiled as an independent crate which makes Rust think methods in
// this file are unused if they are not called by each and every test file :/
#![allow(dead_code)]

use deqs_order_book::{Error, OrderBook, Pair};
use mc_account_keys::AccountKey;
use mc_crypto_ring_signature::Error as RingSignatureError;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_transaction_builder::{
    test_utils::get_input_credentials, EmptyMemoBuilder, ReservedSubaddresses,
    SignedContingentInputBuilder,
};
use mc_transaction_extra::{SignedContingentInput, SignedContingentInputError};
use mc_transaction_types::{Amount, BlockVersion, TokenId};
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
    let block_version = BlockVersion::MAX;
    let fog_resolver = MockFogResolver::default();

    let offerer_account = AccountKey::random(rng);

    let offered_input_credentials = get_input_credentials(
        block_version,
        Amount::new(base_amount, pair.base_token_id),
        &offerer_account,
        &fog_resolver,
        rng,
    );

    let mut builder = SignedContingentInputBuilder::new(
        block_version,
        offered_input_credentials,
        fog_resolver.clone(),
        EmptyMemoBuilder::default(),
    )
    .unwrap();

    builder
        .add_required_output(
            Amount::new(counter_amount, pair.counter_token_id),
            &offerer_account.default_subaddress(),
            rng,
        )
        .unwrap();

    builder.set_tombstone_block(2000);

    builder
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
    let builder = create_sci_builder(pair, base_amount, counter_amount, rng);

    let sci = builder.build(&NoKeysRingSigner {}, rng).unwrap();

    // The contingent input should have a valid signature.
    sci.validate().unwrap();

    sci
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
    let block_version = BlockVersion::MAX;
    let fog_resolver = MockFogResolver::default();

    let offerer_account = AccountKey::random(rng);

    let offered_input_credentials = get_input_credentials(
        block_version,
        Amount::new(base_amount_offered, pair.base_token_id),
        &offerer_account,
        &fog_resolver,
        rng,
    );

    let mut builder = SignedContingentInputBuilder::new(
        block_version,
        offered_input_credentials,
        fog_resolver.clone(),
        EmptyMemoBuilder::default(),
    )
    .unwrap();

    builder
        .add_partial_fill_change_output(
            Amount::new(base_amount_offered, pair.base_token_id),
            &ReservedSubaddresses::from(&offerer_account),
            rng,
        )
        .unwrap();

    if required_base_change_amount > 0 {
        builder
            .add_required_change_output(
                Amount::new(required_base_change_amount, pair.base_token_id),
                &ReservedSubaddresses::from(&offerer_account),
                rng,
            )
            .unwrap();
    }

    // Originator requests an output worth counter_amount to themselves
    builder
        .add_partial_fill_output(
            Amount::new(counter_amount, pair.counter_token_id),
            &offerer_account.default_subaddress(),
            rng,
        )
        .unwrap();

    builder.set_min_partial_fill_value(min_base_fill_amount);

    let sci = builder.build(&NoKeysRingSigner {}, rng).unwrap();
    sci.validate().unwrap();
    sci
}

/// Test order book basic happy flow
pub fn basic_happy_flow(order_book: &impl OrderBook) {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Adding an order should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let order = order_book.add_sci(sci).unwrap();

    let orders = order_book.get_orders(&pair, .., 0).unwrap();
    assert_eq!(orders, vec![order.clone()]);

    // Adding a second order should work
    let sci = create_sci(&pair, 10, 200, &mut rng);
    let order2 = order_book.add_sci(sci).unwrap();

    let orders = order_book.get_orders(&pair, .., 0).unwrap();
    assert_eq!(orders, vec![order.clone(), order2.clone()]);

    // Removing the order by its id should work
    assert_eq!(order, order_book.remove_order_by_id(order.id()).unwrap());

    let orders = order_book.get_orders(&pair, .., 0).unwrap();
    assert_eq!(orders, vec![order2.clone()]);

    // Can't remove the order again
    assert_eq!(
        order_book
            .remove_order_by_id(order.id())
            .unwrap_err()
            .into(),
        Error::OrderNotFound
    );
    assert_eq!(
        order_book
            .remove_orders_by_key_image(&order.sci().key_image())
            .unwrap(),
        vec![],
    );

    // Removing orders by key image should work
    assert_eq!(
        vec![order2.clone()],
        order_book
            .remove_orders_by_key_image(&order2.sci().key_image())
            .unwrap()
    );
    let orders = order_book.get_orders(&pair, .., 0).unwrap();
    assert_eq!(orders, vec![]);

    // Removing orders by tombstone block should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let order1 = order_book.add_sci(sci).unwrap();
    let order1_tombstone = order
        .sci()
        .tx_in
        .input_rules
        .as_ref()
        .unwrap()
        .max_tombstone_block;

    let mut sci_builder = create_sci_builder(&pair, 10, 20, &mut rng);
    sci_builder.set_tombstone_block(order1_tombstone - 1);
    let sci2 = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
    let order2 = order_book.add_sci(sci2).unwrap();

    assert_eq!(
        order_book
            .remove_orders_by_tombstone_block(order1_tombstone - 1)
            .unwrap(),
        vec![order2],
    );

    assert_eq!(
        order_book
            .remove_orders_by_tombstone_block(order1_tombstone - 1)
            .unwrap(),
        vec![],
    );
    assert_eq!(
        order_book
            .remove_orders_by_tombstone_block(order1_tombstone)
            .unwrap(),
        vec![order1],
    );
    assert_eq!(
        order_book
            .remove_orders_by_tombstone_block(order1_tombstone)
            .unwrap(),
        vec![],
    );
}

/// Test some invalid SCI scenarios
pub fn cannot_add_invalid_sci(order_book: &impl OrderBook) {
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
        order_book.add_sci(sci).unwrap_err().into(),
        Error::UnsupportedSci("Unsupported number of required/partial outputs 2/0".into())
    );

    // Make an SCI invalid by messing with the MLSAG
    let mut sci = create_sci(&pair, 10, 20, &mut rng);
    sci.mlsag.responses.pop();

    assert_eq!(
        order_book.add_sci(sci).unwrap_err().into(),
        Error::Sci(SignedContingentInputError::RingSignature(
            RingSignatureError::LengthMismatch(22, 21),
        ))
    );
}

/// Test that get_orders filter correctly.
pub fn get_orders_filtering_works(order_book: &impl OrderBook) {
    let pair1 = pair();
    let pair2 = Pair {
        base_token_id: TokenId::from(10),
        counter_token_id: TokenId::from(2),
    };
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Offer for trading 100 pair1.base tokens into 1000 pair1.counter tokens
    let sci = create_sci(&pair1, 100, 1000, &mut rng);
    let p1_100_for_1000 = order_book.add_sci(sci).unwrap();

    // Offer for partially trading up to 100 pair2.base tokens into 1000
    // pair2.counter tokens
    let sci = create_partial_sci(&pair2, 100, 1, 0, 1000, &mut rng);
    let p2_100_for_1000 = order_book.add_sci(sci).unwrap();

    // Offer for partially trading 5 pair2.base tokens into 50 pair2.counter
    // tokens
    let sci = create_partial_sci(&pair2, 5, 1, 0, 50, &mut rng);
    let p2_5_for_50 = order_book.add_sci(sci).unwrap();

    // Offer for partially trading 50 pair2.base tokens into 5 pair2.counter
    // tokens
    let sci = create_partial_sci(&pair2, 50, 1, 0, 5, &mut rng);
    let p2_50_for_5 = order_book.add_sci(sci).unwrap();

    // Offer for exactly trading 50 pair2.base tokens into 3 pair2.counter
    // tokens
    let sci = create_sci(&pair2, 50, 3, &mut rng);
    let p2_50_for_3 = order_book.add_sci(sci).unwrap();

    // Get all orders at any quantity.
    let orders = order_book.get_orders(&pair1, .., 0).unwrap();
    assert_eq!(orders, vec![p1_100_for_1000.clone()]);

    let orders = order_book.get_orders(&pair2, .., 0).unwrap();
    assert_eq!(
        orders,
        vec![
            p2_50_for_3.clone(),     // rate is 16.6667
            p2_50_for_5.clone(),     // rate is 10
            p2_100_for_1000.clone(), // rate is 0.1
            p2_5_for_50.clone(),     // rate is 0.1
        ]
    );

    // Get all orders but limit to the first 2
    let orders = order_book.get_orders(&pair1, .., 2).unwrap();
    assert_eq!(orders, vec![p1_100_for_1000.clone()]);

    let orders = order_book.get_orders(&pair2, .., 2).unwrap();
    assert_eq!(orders, vec![p2_50_for_3.clone(), p2_50_for_5.clone(),]);

    // Get all orders that can provide an amount that is not available.
    let orders = order_book.get_orders(&pair1, 10000.., 2).unwrap();
    assert_eq!(orders, vec![]);

    let orders = order_book.get_orders(&pair2, 10000.., 2).unwrap();
    assert_eq!(orders, vec![]);

    // Get all orders that can provide a subset of the amount requested.
    let orders = order_book.get_orders(&pair2, 50.., 0).unwrap();
    assert_eq!(
        orders,
        vec![
            p2_50_for_3.clone(),     // rate is 16.6667
            p2_50_for_5.clone(),     // rate is 10
            p2_100_for_1000.clone(), // rate is 0.1
        ]
    );

    let orders = order_book.get_orders(&pair2, 51.., 0).unwrap();
    assert_eq!(
        orders,
        vec![
            p2_100_for_1000.clone(), // rate is 0.1
        ]
    );

    let orders = order_book.get_orders(&pair2, 50..70, 0).unwrap();
    assert_eq!(
        orders,
        vec![
            p2_50_for_3.clone(),     // rate is 16.6667
            p2_50_for_5.clone(),     // rate is 10
            p2_100_for_1000.clone(), // rate is 0.1
        ]
    );
}
