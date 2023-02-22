// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_p2p::libp2p::identity::Keypair;
use deqs_quote_book::{InMemoryQuoteBook, Msg, Quote, RemoveQuoteCallback, SynchronizedQuoteBook};
use deqs_server::{Server, ServerConfig, P2P};
use mc_common::logger::{log, o};
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::AdminServer;
use postage::{
    broadcast,
    prelude::{Sink, Stream},
};
use std::sync::{Arc, Mutex};
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

    // Open the ledger db
    let ledger_db = LedgerDB::open(&config.ledger_db).expect("Could not open ledger db");
    let num_blocks = ledger_db
        .num_blocks()
        .expect("Could not compute num_blocks");
    assert_ne!(0, num_blocks);

    let mut callback_msg_bus_tx = msg_bus_tx.clone();
    // let mut remove_quote_callback = Arc::new(|_|
    // callback_msg_bux_tx.blocking_send(Msg::SciQuoteRemoved(0)).expect(""));
    let remove_quote_callback: RemoveQuoteCallback =
        Arc::new(Mutex::new(move |quotes: Vec<Quote>| {
            for quote in quotes {
                callback_msg_bus_tx
                    .blocking_send(Msg::SciQuoteRemoved(*quote.id()))
                    .unwrap_or_else(|_| {
                        panic!(
                            "Failed to send SCI quote {} removed message to message bus",
                            quote.id()
                        )
                    });
            }
        }));

    // Create quote book
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = Arc::new(SynchronizedQuoteBook::new(
        internal_quote_book,
        ledger_db,
        remove_quote_callback,
        logger.clone(),
    ));

    // Init p2p network
    let (mut p2p, mut p2p_events) = P2P::new(
        synchronized_quote_book.clone(),
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
    )
    .await?;

    // Start GRPC server
    let mut server = Server::new(
        msg_bus_tx,
        synchronized_quote_book,
        config.client_listen_uri.clone(),
        logger.clone(),
    );
    server.start().expect("Failed starting client GRPC server");

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

    // Event loop
    loop {
        select! {
            msg = msg_bus_rx.recv() => {
                match msg {
                    Some(Msg::SciQuoteAdded(quote)) => {
                        if let Err(err) = p2p.broadcast_sci_quote_added(quote).await {
                            log::info!(logger, "broadcast_sci_quote_added failed: {:?}", err)
                        }
                    }

                    Some(Msg::SciQuoteRemoved(quote_id)) => {
                        if let Err(err) = p2p.broadcast_sci_quote_removed(quote_id).await {
                            log::info!(logger, "broadcast_sci_quote_removed failed: {:?}", err)
                        }
                    }

                    None => {
                            log::info!(logger, "msg_bus_rx stream closed");
                            break;
                    }
                }
            }

            event = p2p_events.recv() => {
                match event {
                    Some(event) => p2p.handle_network_event(event).await,
                    None => {
                        log::info!(logger, "p2p_events stream closed");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
