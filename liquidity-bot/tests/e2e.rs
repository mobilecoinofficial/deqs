// Copyright (c) 2023 MobileCoin Inc.

use deqs_api::deqs::{Quote, QuoteStatusCode, SubmitQuotesResponse};
use deqs_liquidity_bot::{
    mini_wallet::{MatchedTxOut, WalletEvent},
    LiquidityBot,
};
use deqs_test_server::DeqsTestServer;
use mc_account_keys::AccountKey;
use mc_blockchain_types::BlockVersion;
use mc_common::logger::{async_test_with_logger, log, Logger};
use mc_ledger_db::test_utils::{add_block_to_ledger, create_ledger, initialize_ledger};
use mc_transaction_core::{ring_signature::KeyImage, Amount, TokenId};
use mc_transaction_extra::SignedContingentInput;
use rand::{rngs::StdRng, RngCore, SeedableRng};
use rust_decimal_macros::dec;
use std::{collections::HashMap, time::Duration};
use tokio_retry::{strategy::FixedInterval, Retry};

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

    let liquidity_bot = LiquidityBot::new(
        account_key.clone(),
        ledger.clone(),
        HashMap::from_iter([(token_id, (TokenId::MOB, dec!(5)))]),
        &deqs_server.client_uri(),
        logger.clone(),
    );

    // Prepare a MatchedTxOut that belongs to the bot's account.
    let amount = Amount::new(100, TokenId::from(5));
    let block_data = add_block_to_ledger(
        &mut ledger,
        block_version,
        &[account_key.default_subaddress()],
        amount,
        &[KeyImage::from(rng.next_u64())],
        &mut rng,
    )
    .unwrap();

    let matched_tx_out = MatchedTxOut {
        tx_out: block_data.contents().outputs[0].clone(),
        amount,
        subaddress_index: 0,
        key_image: KeyImage::from(rng.next_u64()),
    };

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
        received_tx_outs: vec![matched_tx_out.clone()],
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

    //
}
