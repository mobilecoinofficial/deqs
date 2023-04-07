// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_liquidity_bot::{
    mini_wallet::{MiniWallet, WalletEvent},
    update_periodic_metrics, AdminService, Config, LiquidityBot, METRICS_POLL_INTERVAL,
};
use itertools::Itertools;
use mc_common::logger::{log, o};
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::mpsc, time::interval};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sentry_guard = mc_common::sentry::init();
    let config = Config::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    // Open the ledger db
    let ledger_db = LedgerDB::open(&config.ledger_db).expect("Could not open ledger db");
    let num_blocks = ledger_db
        .num_blocks()
        .expect("Could not compute num_blocks");
    assert_ne!(0, num_blocks);

    // Read keyfile
    let account_key =
        mc_util_keyfile::read_keyfile(&config.account_key).expect("Could not read keyfile");

    // Start our mini wallet scanner thingie.
    let (wallet_tx, mut wallet_rx) = mpsc::unbounded_channel();
    let wallet = MiniWallet::new(
        &config.wallet_db,
        ledger_db.clone(),
        account_key.clone(),
        config.first_block_index,
        Arc::new(move |event| {
            wallet_tx
                .send(event)
                .expect("Could not send event to wallet");
        }),
        logger.clone(),
    )
    .expect("Could not create MiniWallet");

    // Start the liquidity bot.
    let pairs = HashMap::from_iter([(
        config.base_token_id,
        (config.counter_token_id, config.swap_rate),
    )]);
    let liquidity_bot =
        LiquidityBot::new(account_key, ledger_db, pairs, &config.deqs, logger.clone());

    // Start admin server
    let config_json = serde_json::to_string(&config).expect("failed to serialize config to JSON");
    let get_config_json = Arc::new(move || Ok(config_json.clone()));
    let bot_admin_service =
        AdminService::new(liquidity_bot.interface().clone(), logger.clone()).into_service();
    let _admin_server = config.admin_listen_uri.as_ref().map(|admin_listen_uri| {
        AdminServer::start(
            None,
            admin_listen_uri,
            "DEQS-Liquidity-Bot".into(),
            "".into(),
            Some(get_config_json),
            vec![bot_admin_service],
            logger.clone(),
        )
        .expect("Failed starting admin server")
    });

    // Since the LiquidityBot itself does not maintain state, feed it any unspent
    // TxOuts we are currently aware of. It will attempt to submit SCIs for all
    // of them, and if existing ones are already present on the DEQS for the
    // same TxOuts (identified by their KeyImage), it will use those instead for
    // future resubmits. It is safe to not include spent tx outs here since the
    // TxOuts held inside the wallet's state, at the moment they are grabbed,
    // are guaranteed to be unspent. If they are spent after the state is grabbed,
    // the wallet will notify the bot of the spend using the wallet_tx/rx channel.
    let existing_tx_outs = wallet.matched_tx_outs();
    log::info!(
        logger,
        "Feeding {} existing TxOuts to liquidity bot",
        existing_tx_outs.len()
    );
    for (block_index, received_tx_outs) in &existing_tx_outs
        .into_iter()
        .group_by(|mtxo| mtxo.block_index)
    {
        liquidity_bot
            .interface()
            .notify_wallet_event(WalletEvent::BlockProcessed {
                block_index,
                received_tx_outs: received_tx_outs.collect(),
                spent_tx_outs: vec![],
            });
    }

    // Event loop.
    let mut metrics_interval = interval(METRICS_POLL_INTERVAL);
    metrics_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!(logger, "Ctrl-C received, exiting");
                break;
            }

            event = wallet_rx.recv() => {
                match event {
                    Some(event) => {
                        log::trace!(logger, "Wallet event: {:?}", event);
                        liquidity_bot.interface().notify_wallet_event(event);
                    }
                   None => {
                        log::info!(logger, "Wallet event channel closed");
                        break;
                    }
                }
            }

            _ = metrics_interval.tick() => {
                update_periodic_metrics(&wallet, liquidity_bot.interface()).await;
            }
        }
    }

    Ok(())
}
