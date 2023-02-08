// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, GrpcServer, Msg, P2P};
use deqs_api::DeqsClientUri;
use deqs_p2p::libp2p::{identity::Keypair, Multiaddr};
use deqs_quote_book::QuoteBook;
use mc_common::logger::{log, Logger};
use postage::{broadcast, prelude::Stream};
use tokio::{select, sync::mpsc};

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

pub struct DeqsServer<QB: QuoteBook> {
    /// Shutdown sender, used to signal the event loop to shutdown.
    shutdown_tx: mpsc::UnboundedSender<()>,

    /// Shutdown acknowledged receiver.
    shutdown_ack_rx: mpsc::UnboundedReceiver<()>,

    /// Must hold a reference to the grpc server, otherwise it will be dropped.
    grpc_server: GrpcServer<QB>,
}

impl<QB: QuoteBook> DeqsServer<QB> {
    pub async fn start(
        quote_book: QB,
        grpc_listen_address: DeqsClientUri,
        p2p_bootstrap_peers: Vec<Multiaddr>,
        p2p_listen_address: Option<Multiaddr>,
        p2p_external_address: Option<Multiaddr>,
        p2p_keypair: Option<Keypair>,
        logger: Logger,
    ) -> Result<Self, Error> {
        let (msg_bus_tx, mut msg_bus_rx) = broadcast::channel::<Msg>(MSG_BUS_QUEUE_SIZE);
        let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel();
        let (shutdown_ack_tx, shutdown_ack_rx) = mpsc::unbounded_channel();

        // Init p2p network
        let (mut p2p, mut p2p_events) = P2P::new(
            quote_book.clone(),
            p2p_bootstrap_peers,
            p2p_listen_address,
            p2p_external_address,
            p2p_keypair,
            logger.clone(),
        )
        .await?;

        // Start GRPC server
        let mut grpc_server =
            GrpcServer::new(msg_bus_tx, quote_book, grpc_listen_address, logger.clone());
        grpc_server
            .start()
            .expect("Failed starting client GRPC server");

        // Event loop
        tokio::spawn(async move {
            log::info!(logger, "DeqsServer event loop started");

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

                    _ = shutdown_rx.recv() => {
                        log::info!(logger, "DeqsServer shutdown requested");
                        break
                    }

                }
            }

            // This will cause the receiver to become ready (and return None when recv() is
            // called)
            drop(shutdown_ack_tx);
        });

        Ok(Self {
            shutdown_tx,
            shutdown_ack_rx,
            grpc_server,
        })
    }

    pub async fn shutdown(&mut self) {
        if self.shutdown_tx.send(()).is_err() {
            // Shutdown already requested
            return;
        }

        // Wait for the event loop to drop the ack sender.
        let _ = self.shutdown_ack_rx.recv().await;
    }

    pub fn grpc_listen_uri(&self) -> Option<DeqsClientUri> {
        self.grpc_server.actual_listen_uri()
    }
}
