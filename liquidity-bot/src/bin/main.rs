// Copyright (c) 2023 MobileCoin Inc.

use std::{collections::HashMap, sync::Arc};

use clap::Parser;
use deqs_liquidity_bot::{
    mini_wallet::{MiniWallet, WalletEvent},
    Config, LiquidityBot,
};
use mc_common::logger::{log, o};
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use tokio::sync::mpsc;

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

    // Start admin server
    let config_json = serde_json::to_string(&config).expect("failed to serialize config to JSON");
    let get_config_json = Arc::new(move || Ok(config_json.clone()));
    let _admin_server = config.admin_listen_uri.as_ref().map(|admin_listen_uri| {
        AdminServer::start(
            None,
            admin_listen_uri,
            "DEQS-Liquidity-Bot".into(),
            "".into(),
            Some(get_config_json),
            logger.clone(),
        )
        .expect("Failed starting admin server")
    });

    // TODO
    let (wallet_tx, mut wallet_rx) = mpsc::unbounded_channel();
    let _wallet = MiniWallet::new(
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

    let pairs = HashMap::from_iter([(
        config.base_token_id,
        (config.counter_token_id, config.swap_rate),
    )]);
    let liquidity_bot =
        LiquidityBot::new(account_key, ledger_db, pairs, &config.deqs, logger.clone());

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!(logger, "Ctrl-C received, exiting");
                break;
            }

            event = wallet_rx.recv() => {
                match event {
                    Some(event) => {
                        log::debug!(logger, "Wallet event: {:?}", event);
                        liquidity_bot.notify_wallet_event(event);
                    }
                   None => {
                        log::info!(logger, "Wallet event channel closed");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
