// Copyright (c) 2023 MobileCoin Inc.

use deqs_order_book::OrderBook;
use mc_account_keys::AccountKey;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_transaction_builder::{
    test_utils::get_input_credentials, EmptyMemoBuilder, SignedContingentInputBuilder,
};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::{Amount, BlockVersion, TokenId};
use rand::{rngs::StdRng, SeedableRng};
use rand_core::{CryptoRng, RngCore};

/// Create an SCI builder that offers some amount of a given token in exchange
/// for a different amount of another token. Returning the builder allows the
/// caller to customize the SCI further.
pub fn create_sci_builder(
    offered_token_id: TokenId,
    offered_amount: u64,
    priced_in_token_id: TokenId,
    cost: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInputBuilder<MockFogResolver> {
    let block_version = BlockVersion::MAX;
    let fog_resolver = MockFogResolver::default();

    let offerer_account = AccountKey::random(rng);

    let offered_input_credentials = get_input_credentials(
        block_version,
        Amount::new(offered_amount, offered_token_id),
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
            Amount::new(cost, priced_in_token_id),
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
    offered_token_id: TokenId,
    offered_amount: u64,
    priced_in_token_id: TokenId,
    cost: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInput {
    let builder = create_sci_builder(
        offered_token_id,
        offered_amount,
        priced_in_token_id,
        cost,
        rng,
    );

    let sci = builder.build(&NoKeysRingSigner {}, rng).unwrap();

    // The contingent input should have a valid signature.
    sci.validate().unwrap();

    sci
}

/// Test order book basic happy flow
pub fn basic_happy_flow<OB: OrderBook>(order_book: &OB, not_found_err_variant: OB::Error) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Adding an order should work
    let sci = create_sci(TokenId::from(1), 10, TokenId::from(2), 20, &mut rng);
    let order = order_book.add_sci(sci).unwrap();

    let orders = order_book
        .get_orders(TokenId::from(1), .., TokenId::from(2), ..)
        .unwrap();
    assert_eq!(orders, vec![order.clone()]);

    // Adding a second order should work
    let sci = create_sci(TokenId::from(1), 10, TokenId::from(2), 200, &mut rng);
    let order2 = order_book.add_sci(sci).unwrap();

    let orders = order_book
        .get_orders(TokenId::from(1), .., TokenId::from(2), ..)
        .unwrap();
    assert_eq!(orders, vec![order.clone(), order2.clone()]);

    // Removing the order by its id should work
    assert_eq!(order, order_book.remove_order_by_id(order.id()).unwrap());

    let orders = order_book
        .get_orders(TokenId::from(1), .., TokenId::from(2), ..)
        .unwrap();
    assert_eq!(orders, vec![order2.clone()]);

    // Can't remove the order again
    assert_eq!(
        order_book.remove_order_by_id(order.id()),
        Err(not_found_err_variant)
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
}

/// Test some invalid SCI scenarios
pub fn cannot_add_invalid_sci<OB: OrderBook>(
    order_book: &OB,
    incorrect_number_of_outputs_err_variant: OB::Error,
    invalid_signature_err_variant: OB::Error,
) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Make an SCI invalid by adding some random required output
    let mut sci_builder = create_sci_builder(TokenId::from(1), 10, TokenId::from(2), 20, &mut rng);
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
        order_book.add_sci(sci),
        Err(incorrect_number_of_outputs_err_variant)
    );

    // Make an SCI invalid by messing with the MLSAG
    let mut sci = create_sci(TokenId::from(1), 10, TokenId::from(2), 20, &mut rng);
    sci.mlsag.responses.pop();

    assert_eq!(
        order_book.add_sci(sci),
        Err(invalid_signature_err_variant)
    );
}
