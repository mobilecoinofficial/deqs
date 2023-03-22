// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_liquidity_bot::{mini_wallet::MiniWallet, Config};
use mc_common::logger::o;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use std::sync::Arc;
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
            vec![],
            logger.clone(),
        )
        .expect("Failed starting admin server")
    });

    // Start our mini wallet scanner thingie.
    let (wallet_tx, wallet_rx) = mpsc::unbounded_channel();
    let _wallet = MiniWallet::new(
        &config.wallet_db,
        ledger_db,
        account_key,
        config.first_block_index,
        Arc::new(move |event| {
            wallet_tx
                .send(event)
                .expect("Could not send event to wallet");
        }),
        logger,
    )
    .expect("Could not create MiniWallet");

    drop(wallet_rx); // This gets used in a followup PR

    todo!("bot implementation is in a followup pr");
}
