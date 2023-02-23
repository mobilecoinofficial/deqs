// Copyright (c) 2023 MobileCoin Inc.

use crate::{update_periodic_metrics, Error, GrpcServer, Msg, METRICS_POLL_INTERVAL, P2P};
use deqs_api::DeqsClientUri;
use deqs_p2p::libp2p::{identity::Keypair, Multiaddr};
use deqs_quote_book_api::{Quote, QuoteBook};
use mc_common::logger::{log, Logger};
use postage::{
    broadcast::{Receiver, Sender},
    prelude::{Sink, Stream},
};
use tokio::{
    select,
    sync::mpsc,
    time::{interval, MissedTickBehavior},
};

pub struct Server<QB: QuoteBook> {
    /// Shutdown sender, used to signal the event loop to shutdown.
    shutdown_tx: mpsc::UnboundedSender<()>,

    /// Shutdown acknowledged receiver.
    shutdown_ack_rx: mpsc::UnboundedReceiver<()>,

    /// Must hold a reference to the grpc server, otherwise it will be dropped.
    grpc_server: GrpcServer<QB>,

    /// Addresses the peer to peer network is listening on.
    p2p_listen_addrs: Vec<Multiaddr>,
}

impl<QB: QuoteBook> Server<QB> {
    pub async fn start(
        quote_book: QB,
        grpc_listen_address: DeqsClientUri,
        p2p_bootstrap_peers: Vec<Multiaddr>,
        p2p_listen_address: Option<Multiaddr>,
        p2p_external_address: Option<Multiaddr>,
        p2p_keypair: Option<Keypair>,
        msg_bus_tx: Sender<Msg>,
        mut msg_bus_rx: Receiver<Msg>,
        logger: Logger,
    ) -> Result<Self, Error> {
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

        // Get p2p listening addresses
        let p2p_listen_addrs = p2p.listen_addrs().await?;

        // Start GRPC server
        let mut grpc_server = GrpcServer::new(
            msg_bus_tx,
            quote_book.clone(),
            grpc_listen_address,
            logger.clone(),
        );
        grpc_server
            .start()
            .expect("Failed starting client GRPC server");

        // Event loop
        let mut metrics_interval = interval(METRICS_POLL_INTERVAL);
        metrics_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        tokio::spawn(async move {
            log::info!(logger, "Server event loop started");

            loop {
                select! {
                    msg = msg_bus_rx.recv() => {
                        match msg {
                            Some(Msg::SciQuoteAdded(quote)) => {
                                if let Err(err) = p2p.broadcast_sci_quote_added(quote).await {
                                    log::info!(logger, "broadcast_sci_quote_added failed: {:?}", err)
                                }
                            }

                            Some(Msg::SciQuoteRemoved(quote)) => {
                                if let Err(err) = p2p.broadcast_sci_quote_removed(*quote.id()).await {
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
                        log::info!(logger, "Server shutdown requested");
                        break
                    }

                    _ = metrics_interval.tick() => {
                        if let Err(err) = update_periodic_metrics(&quote_book, &p2p).await {
                            log::error!(logger, "update_periodic_metrics failed: {:?}", err)
                        }
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
            p2p_listen_addrs,
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

    pub fn p2p_listen_addrs(&self) -> Vec<Multiaddr> {
        self.p2p_listen_addrs.clone()
    }
    pub fn get_remove_quote_callback_function(
        mut callback_msg_bus_tx: Sender<Msg>,
    ) -> Box<dyn FnMut(Vec<Quote>) + Send + Sync> {
        Box::new(move |quotes: Vec<Quote>| {
            for quote in quotes {
                callback_msg_bus_tx
                    .blocking_send(Msg::SciQuoteRemoved(quote.clone()))
                    .unwrap_or_else(|_| {
                        panic!(
                            "Failed to send SCI quote {} removed message to
        message bus",
                            quote.id()
                        )
                    });
            }
        })
    }
}
