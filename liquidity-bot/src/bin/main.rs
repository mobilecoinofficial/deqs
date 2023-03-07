// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_liquidity_bot::Config;
use mc_common::logger::o;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use std::sync::Arc;

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
    drop(account_key); // We will use it in a followup PR

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

    todo!("bot implementation is in a followup pr");
}
