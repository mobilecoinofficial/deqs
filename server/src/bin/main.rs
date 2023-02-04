// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_p2p::libp2p::identity::Keypair;
use deqs_quote_book::{InMemoryQuoteBook, SynchronizedQuoteBook};
use deqs_server::{Msg, Server, ServerConfig, P2P};
use mc_common::logger::{log, o};
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use postage::{broadcast, prelude::Stream};
use std::sync::Arc;
use tokio::select;

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    let (msg_bus_tx, mut msg_bus_rx) = broadcast::channel::<Msg>(MSG_BUS_QUEUE_SIZE);

    let (_p2p, mut p2p_events) = P2P::new(
        quote_book.clone(),
        config.p2p_bootstrap_peers.clone(),
        config.p2p_listen.clone(),
        config.p2p_external_address.clone(),
        config
            .p2p_keypair_path
            .as_ref()
            .map(|path| -> Result<Keypair, Box<dyn std::error::Error>> {
                let bytes = std::fs::read(path)?;
                Ok(Keypair::from_protobuf_encoding(&bytes)?)
            })
            .transpose()?,
        logger.clone(),
    )?;

    // Open the ledger db
    let ledger_db = LedgerDB::open(&config.ledger_db).expect("Could not open ledger db");
    let num_blocks = ledger_db
        .num_blocks()
        .expect("Could not compute num_blocks");
    assert_ne!(0, num_blocks);

    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger_db);

    let mut server = Server::new(
        msg_bus_tx,
        synchronized_quote_book,
        config.client_listen_uri.clone(),
        logger.clone(),
    );
    server.start().expect("Failed starting client GRPC server");

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

    loop {
        select! {
            _ = msg_bus_rx.recv() => {
                // This allows us to ensure we always have at least 1 receiver on the message
                // bus, which will prevent sends from failing.
            }

            event = p2p_events.recv() => {
                log::debug!(logger, "p2p event: {:?}", event);
            }
        }
    }
}
