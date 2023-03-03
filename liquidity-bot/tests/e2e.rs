// Copyright (c) 2023 MobileCoin Inc.

use deqs_api::deqs::{Quote, QuoteStatusCode, SubmitQuotesResponse};
use deqs_liquidity_bot::{
    mini_wallet::{MatchedTxOut, WalletEvent},
    LiquidityBot,
};
use deqs_test_server::DeqsTestServer;
use mc_account_keys::AccountKey;
use mc_blockchain_types::BlockVersion;
use mc_common::logger::{async_test_with_logger, Logger};
use mc_ledger_db::test_utils::{
    add_txos_and_key_images_to_ledger, create_ledger, initialize_ledger,
};
use mc_transaction_core::{ring_signature::KeyImage, Amount, TokenId};
use mc_transaction_core_test_utils::get_outputs;
use mc_transaction_extra::SignedContingentInput;
use rand::{rngs::StdRng, RngCore, SeedableRng};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::{collections::HashMap, time::Duration};
use tokio_retry::{strategy::FixedInterval, Retry};

/// Test that out of 3 matched TxOuts, only the one with the token id the bot
/// cares about gets submitted to the DEQS, and that the other two are ignored.
/// Also verify that the partial output is present and contains the correct
/// amount and token id.
#[async_test_with_logger(flavor = "multi_thread")]
async fn test_basic_submission(logger: Logger) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let account_key = AccountKey::random(&mut rng);

    let block_version = BlockVersion::MAX;
    let mut ledger = create_ledger();

    let n_blocks = 3;
    initialize_ledger(block_version, &mut ledger, n_blocks, &account_key, &mut rng);

    let deqs_server = DeqsTestServer::start(logger.clone());
    let token_id = TokenId::from(5);
    let swap_rate = dec!(5);

    let liquidity_bot = LiquidityBot::new(
        account_key.clone(),
        ledger.clone(),
        HashMap::from_iter([(token_id, (TokenId::MOB, swap_rate))]),
        &deqs_server.client_uri(),
        logger.clone(),
    );

    // Prepare three MatchedTxOut that belongs to the bot's account.
    // One will be for the token id the bot is offering, the other two should get
    // ignored.
    let amount = Amount::new(100, TokenId::from(5));

    let recipient = account_key.default_subaddress();
    let recipient_and_amount = vec![
        (recipient.clone(), Amount::new(100, TokenId::from(100))),
        (recipient.clone(), amount),
        (recipient.clone(), Amount::new(100, TokenId::MOB)),
    ];
    let outputs = get_outputs(block_version, &recipient_and_amount, &mut rng);

    let block_data = add_txos_and_key_images_to_ledger(
        &mut ledger,
        block_version,
        outputs,
        vec![KeyImage::from(rng.next_u64())],
        &mut rng,
    )
    .unwrap();

    let matched_tx_outs = block_data
        .contents()
        .outputs
        .iter()
        .cloned()
        .enumerate()
        .map(|(idx, tx_out)| MatchedTxOut {
            tx_out,
            amount: recipient_and_amount[idx].1,
            subaddress_index: 0,
            key_image: KeyImage::from(rng.next_u64()),
        })
        .collect::<Vec<_>>();

    // The actual matched_tx_out we care about is the one that has the amount with
    // the token the bot is offering
    let matched_tx_out = &matched_tx_outs[1];

    // Set up the server response to Stale so that the bot doesn't keep trying to
    // re-submit.
    let resp = SubmitQuotesResponse {
        quotes: vec![Quote::default()].into(),
        status_codes: vec![QuoteStatusCode::QUOTE_IS_STALE].into(),
        error_messages: vec!["".into()].into(),
        ..Default::default()
    };
    deqs_server.set_submit_quotes_response(Ok(resp));

    // Feed the bot a block with a TxOut belonging to its account.
    liquidity_bot.notify_wallet_event(WalletEvent::BlockProcessed {
        block_index: block_data.block().index,
        received_tx_outs: matched_tx_outs.clone(),
        spent_tx_outs: vec![],
    });

    // Wait for the bot to process the block. We expect it to submit an SCI to
    // the test server.
    let retry_strategy = FixedInterval::new(Duration::from_millis(100)).take(50);
    let req = Retry::spawn(retry_strategy, || async {
        let reqs = deqs_server.pop_submit_quotes_requests();
        match reqs.len() {
            0 => Err("no requests"),
            1 => Ok(reqs[0].clone()),
            _ => panic!("too many requests"),
        }
    })
    .await
    .unwrap();

    // The SCI should point at the TxOut we handed to the bot.
    let sci = SignedContingentInput::try_from(&req.quotes[0]).unwrap();
    assert!(sci.tx_in.ring.contains(&matched_tx_out.tx_out));

    // The SCI should have a partial fill output targetted at the correct account,
    // with the correct amount.
    let input_rules = sci.tx_in.input_rules.as_ref().unwrap();
    assert_eq!(input_rules.partial_fill_outputs.len(), 1);
    let (partial_fill_amount, _) = input_rules.partial_fill_outputs[0].reveal_amount().unwrap();
    assert_eq!(partial_fill_amount.token_id, TokenId::MOB);
    assert_eq!(
        Decimal::from(partial_fill_amount.value),
        Decimal::from(amount.value) * swap_rate
    );
}
