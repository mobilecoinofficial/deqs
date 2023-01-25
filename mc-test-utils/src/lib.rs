// Copyright (c) 2023 MobileCoin Inc.

//! Test utilities for generating some MobileCoin objects

use mc_account_keys::AccountKey;
use mc_crypto_ring_signature_signer::NoKeysRingSigner;
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_transaction_builder::{
    test_utils::get_input_credentials, EmptyMemoBuilder, ReservedSubaddresses,
    SignedContingentInputBuilder,
};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::{Amount, BlockVersion, TokenId};
use rand_core::{CryptoRng, RngCore};

/// Create an SCI builder that offers some amount of a given token in exchange
/// for a different amount of another token. Returning the builder allows the
/// caller to customize the SCI further.
pub fn create_sci_builder(
    base_token_id: TokenId,
    counter_token_id: TokenId,
    base_amount: u64,
    counter_amount: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInputBuilder<MockFogResolver> {
    let block_version = BlockVersion::MAX;
    let fog_resolver = MockFogResolver::default();

    let offerer_account = AccountKey::random(rng);

    let offered_input_credentials = get_input_credentials(
        block_version,
        Amount::new(base_amount, base_token_id),
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
            Amount::new(counter_amount, counter_token_id),
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
    base_token_id: TokenId,
    counter_token_id: TokenId,
    base_amount: u64,
    counter_amount: u64,
    rng: &mut (impl RngCore + CryptoRng),
) -> SignedContingentInput {
    let builder = create_sci_builder(
        base_token_id,
        counter_token_id,
        base_amount,
        counter_amount,
        rng,
    );

    let sci = builder.build(&NoKeysRingSigner {}, rng).unwrap();

    // The contingent input should have a valid signature.
    sci.validate().unwrap();

    sci
}

/// Create a partial fill SCI that offers between required_base_change_amount
/// and base_amount_offered tokens, with a minimum required fill of
/// min_base_fill_amount.
pub fn create_partial_sci(
    base_token_id: TokenId,
    counter_token_id: TokenId,
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
        Amount::new(base_amount_offered, base_token_id),
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
            Amount::new(base_amount_offered, base_token_id),
            &ReservedSubaddresses::from(&offerer_account),
            rng,
        )
        .unwrap();

    if required_base_change_amount > 0 {
        builder
            .add_required_change_output(
                Amount::new(required_base_change_amount, base_token_id),
                &ReservedSubaddresses::from(&offerer_account),
                rng,
            )
            .unwrap();
    }

    // Originator requests an output worth counter_amount to themselves
    builder
        .add_partial_fill_output(
            Amount::new(counter_amount, counter_token_id),
            &offerer_account.default_subaddress(),
            rng,
        )
        .unwrap();

    builder.set_min_partial_fill_value(min_base_fill_amount);

    let sci = builder.build(&NoKeysRingSigner {}, rng).unwrap();
    sci.validate().unwrap();
    sci
}
