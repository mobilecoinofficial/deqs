// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_p2p::libp2p::identity::Keypair;
use deqs_quote_book_in_memory::InMemoryQuoteBook;
use deqs_quote_book_sqlite::SqliteQuoteBook;
use deqs_quote_book_synchronized::{RemoveQuoteCallback, SynchronizedQuoteBook};
use deqs_server::{Msg, Server, ServerConfig};
use mc_common::logger::o;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use postage::broadcast;
use std::sync::Arc;

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    // Open the ledger db
    let ledger_db = LedgerDB::open(&config.ledger_db).expect("Could not open ledger db");
    let num_blocks = ledger_db
        .num_blocks()
        .expect("Could not compute num_blocks");
    assert_ne!(0, num_blocks);

    // Read keypair, if provided.
    let keypair = config
        .p2p_keypair_path
        .as_ref()
        .map(|path| -> Result<Keypair, Box<dyn std::error::Error>> {
            let bytes = std::fs::read(path)?;
            Ok(Keypair::from_protobuf_encoding(&bytes)?)
        })
        .transpose()?;
    let (msg_bus_tx, msg_bus_rx) = broadcast::channel::<Msg>(MSG_BUS_QUEUE_SIZE);

    let remove_quote_callback: RemoveQuoteCallback = Server::<
        SynchronizedQuoteBook<InMemoryQuoteBook, LedgerDB>,
    >::get_remove_quote_callback_function(
        msg_bus_tx.clone()
    );

    // Create quote book
    let internal_quote_book = SqliteQuoteBook::new_from_file_path(
        &config.db_path,
        10,
        InMemoryQuoteBook::default(),
        logger.clone(),
    )
    .expect("failed initializing database");
    let synchronized_quote_book = SynchronizedQuoteBook::new(
        internal_quote_book,
        ledger_db,
        remove_quote_callback,
        logger.clone(),
    );

    // Start admin server
    let config_json = serde_json::to_string(&config).expect("failed to serialize config to JSON");
    let get_config_json = Arc::new(move || Ok(config_json.clone()));
    let id = config.client_listen_uri.to_string();
    let _admin_server = config.admin_listen_uri.as_ref().map(|admin_listen_uri| {
        AdminServer::start(
            None,
            admin_listen_uri,
            "DEQS".into(),
            id,
            Some(get_config_json),
            logger.clone(),
        )
        .expect("Failed starting admin server")
    });

    // Start deqs server. Stays alive as long as it remains in scope.
    let _deqs_server = Server::start(
        synchronized_quote_book,
        config.client_listen_uri,
        config.p2p_bootstrap_peers,
        config.p2p_listen,
        config.p2p_external_address,
        keypair,
        msg_bus_tx,
        msg_bus_rx,
        config.quote_minimum_map,
        logger.clone(),
    )
    .await?;

    // Wait until we are asked to quit.
    tokio::signal::ctrl_c().await?;

    Ok(())
}
