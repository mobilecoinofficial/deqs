// Copyright (c) 2023 MobileCoin Inc.

mod rpc;

pub use rpc::RpcError;

use crate::Error;
use deqs_p2p::{
    libp2p::{
        gossipsub::{GossipsubMessage, IdentTopic},
        identity::Keypair,
        request_response::ResponseChannel,
        Multiaddr, PeerId,
    },
    Behaviour, Client, Network, NetworkBuilder, NetworkEvent, NetworkEventLoopHandle,
};
use deqs_quote_book::{Error as QuoteBookError, Quote, QuoteBook, QuoteId};
use futures::executor::block_on;
use mc_common::logger::{log, Logger};
use rpc::{Request, Response, RpcClient};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Debug, Deserialize, Serialize)]
pub enum GossipMsgBusData {
    SciQuoteAdded(Quote),
    SciQuoteRemoved(QuoteId),
}

pub struct P2P<QB: QuoteBook> {
    quote_book: QB,
    logger: Logger,
    client: Client<Request, Response>,
    rpc: RpcClient,
    event_loop_handle: NetworkEventLoopHandle,
    msg_bus_topic: IdentTopic,
}

impl<QB: QuoteBook> P2P<QB> {
    pub async fn new(
        quote_book: QB,
        bootstrap_peers: Vec<Multiaddr>,
        listen_addr: Option<Multiaddr>,
        external_addr: Option<Multiaddr>,
        keypair: Option<Keypair>,
        logger: Logger,
    ) -> Result<(Self, UnboundedReceiver<NetworkEvent<Request, Response>>), Error> {
        let msg_bus_topic = IdentTopic::new("mc/deqs/server/msg-bus");

        let keypair = keypair.unwrap_or_else(Keypair::generate_ed25519);

        let behaviour = Behaviour::<Request, Response>::new(&keypair)?;
        let mut network_builder =
            NetworkBuilder::new(keypair, behaviour, bootstrap_peers, logger.clone())?;
        if let Some(ref listen_addr) = listen_addr {
            network_builder = network_builder.listen_address(listen_addr.clone());
        }
        if let Some(ref external_addr) = external_addr {
            network_builder = network_builder.external_addresses(vec![external_addr.clone()]);
        }
        let Network {
            event_loop_handle,
            events,
            mut client,
        } = network_builder.build()?;

        client.subscribe_gossip(msg_bus_topic.clone()).await?;

        let rpc = RpcClient::new(client.clone(), logger.clone());

        Ok((
            Self {
                quote_book,
                logger,
                client,
                rpc,
                event_loop_handle,
                msg_bus_topic,
            },
            events,
        ))
    }

    pub async fn broadcast_sci_quote_added(&mut self, quote: Quote) -> Result<(), Error> {
        let bytes = mc_util_serial::serialize(&GossipMsgBusData::SciQuoteAdded(quote))?;
        let _message_id = self
            .client
            .publish_gossip(self.msg_bus_topic.clone(), bytes)
            .await?;
        Ok(())
    }

    pub async fn broadcast_sci_quote_removed(&mut self, quote_id: QuoteId) -> Result<(), Error> {
        let bytes = mc_util_serial::serialize(&GossipMsgBusData::SciQuoteRemoved(quote_id))?;
        let _message_id = self
            .client
            .publish_gossip(self.msg_bus_topic.clone(), bytes)
            .await?;
        Ok(())
    }

    pub async fn handle_connection_established(&mut self, peer_id: PeerId) -> Result<(), Error> {
        log::info!(self.logger, "Connection established with peer: {}", peer_id);

        // Start a task to sync quotes from the peer.
        let mut rpc = self.rpc.clone();
        let quote_book = self.quote_book.clone();
        let logger = self.logger.clone();
        tokio::spawn(async move {
            if let Err(err) = sync_quotes_from_peer(peer_id, &mut rpc, &quote_book, &logger).await {
                log::warn!(logger, "Failed to sync quotes from peer: {}", err);
            }
        });

        Ok(())
    }

    pub async fn handle_rpc_request(
        &mut self,
        _peer_id: PeerId,
        request: Request,
        channel: ResponseChannel<Response>,
    ) -> Result<(), Error> {
        let response = match request {
            Request::GetAllQuoteIds => self
                .quote_book
                .get_quote_ids(None)
                .map(Response::AllQuoteIds)
                .unwrap_or_else(|err| Response::Error(RpcError::QuoteBook(err))),

            Request::GetQuoteById(quote_id) => self
                .quote_book
                .get_quote_by_id(&quote_id)
                .map(Response::MaybeQuote)
                .unwrap_or_else(|err| Response::Error(RpcError::QuoteBook(err))),
        };

        self.client.rpc_response(response, channel).await?;

        Ok(())
    }

    pub async fn handle_gossip_message(&mut self, message: GossipsubMessage) -> Result<(), Error> {
        if message.topic == self.msg_bus_topic.hash() {
            let msg: GossipMsgBusData = mc_util_serial::deserialize(&message.data)?;
            self.handle_msg_bus_message(msg).await?;
        } else {
            log::warn!(
                self.logger,
                "Received gossip message with unknown topic: {:?}",
                message.topic
            );
        }
        Ok(())
    }

    async fn handle_msg_bus_message(&mut self, msg: GossipMsgBusData) -> Result<(), Error> {
        match msg {
            GossipMsgBusData::SciQuoteAdded(quote) => self.handle_sci_quote_added(quote).await,

            GossipMsgBusData::SciQuoteRemoved(quote_id) => {
                self.handle_sci_quote_removed(&quote_id).await
            }
        }
    }

    async fn handle_sci_quote_added(&mut self, remote_quote: Quote) -> Result<(), Error> {
        let local_quote = self
            .quote_book
            .add_sci(remote_quote.sci().clone(), Some(remote_quote.timestamp()))?;

        // Sanity
        if remote_quote != local_quote {
            log::warn!(self.logger, "Received quote via gossip that did not match local quote generated from the quote SCI: {:?} vs {:?}", remote_quote, local_quote);
            return Ok(());
        }

        log::info!(self.logger, "Added quote via gossip: {}", local_quote.id());

        Ok(())
    }

    async fn handle_sci_quote_removed(&mut self, quote_id: &QuoteId) -> Result<(), Error> {
        match self.quote_book.remove_quote_by_id(quote_id) {
            Ok(_) => {
                log::info!(self.logger, "Removed quote via gossip: {}", quote_id,);
            }
            Err(err) => {
                log::info!(
                    self.logger,
                    "Failed removing quote {} via gossip: {:?}",
                    quote_id,
                    err,
                );
            }
        }

        Ok(())
    }
}

impl<QB: QuoteBook> Drop for P2P<QB> {
    fn drop(&mut self) {
        block_on(self.event_loop_handle.shutdown());
    }
}

// TODO we might want to have an always-running task that syncs quotes from
// peers sequentially, rather than spawning a new task for each peer.
// What happens right now is that we connect to multiple peers at the same time
// and end up syncing a lot of identical quotes from each of the peers.
async fn sync_quotes_from_peer(
    peer_id: PeerId,
    rpc: &mut RpcClient,
    quote_book: &impl QuoteBook,
    logger: &Logger,
) -> Result<(), Error> {
    // Get all quote ids from peer.
    let remote_quote_ids = BTreeSet::from_iter(rpc.get_all_quote_ids(peer_id).await?);

    // Get all local quote ids, and find which ones we are missing.
    let local_quote_ids = BTreeSet::from_iter(quote_book.get_quote_ids(None)?);
    let missing_quote_ids: Vec<_> = remote_quote_ids.difference(&local_quote_ids).collect();

    log::info!(
        logger,
        "Received {} quote ids from peer {:?}. Will need to sync {} quotes",
        remote_quote_ids.len(),
        peer_id,
        missing_quote_ids.len(),
    );

    let mut num_added_quotes = 0;
    let mut num_duplicate_quotes = 0;
    let mut num_errors = 0;
    let mut num_missing_quotes = 0;

    for (i, quote_id) in missing_quote_ids.iter().enumerate() {
        log::debug!(
            logger,
            "Syncing quote {}/{} from peer {:?}: {}",
            i + 1,
            missing_quote_ids.len(),
            peer_id,
            quote_id,
        );

        match rpc.get_quote_by_id(peer_id, **quote_id).await {
            Ok(Some(quote)) => {
                match quote_book.add_sci(quote.sci().clone(), Some(quote.timestamp())) {
                    Ok(_) => {
                        log::debug!(
                            logger,
                            "Synced quote {} ({}/{}) from peer {:?}",
                            quote_id,
                            i + 1,
                            missing_quote_ids.len(),
                            peer_id
                        );

                        num_added_quotes += 1;
                    }

                    Err(QuoteBookError::QuoteAlreadyExists) => {
                        log::debug!(
                            logger,
                            "Failed to add quote {} ({}/{}) from peer {:?}: Already exists (this is acceptable)",
                            quote_id,
                            i + 1,
                            missing_quote_ids.len(),
                            peer_id,
                        );

                        num_duplicate_quotes += 1;
                    }

                    Err(err) => {
                        log::info!(
                            logger,
                            "Failed to add quote {} ({}/{}) from peer {:?}: {:?}",
                            quote_id,
                            i + 1,
                            missing_quote_ids.len(),
                            peer_id,
                            err,
                        );

                        num_errors += 1;
                    }
                }
            }

            Ok(None) => {
                log::debug!(logger, "Peer {:?} did not have quote {}", peer_id, quote_id,);
                num_missing_quotes += 1;
            }

            Err(err) => {
                log::warn!(
                    logger,
                    "Failed to sync quote {} from peer {:?}: {:?}",
                    quote_id,
                    peer_id,
                    err,
                );

                num_errors += 1;
            }
        }
    }

    log::info!(
        logger,
        "Synced {} quotes from peer {:?}: {} added, {} duplicates, {} missing, {} errors",
        missing_quote_ids.len(),
        peer_id,
        num_added_quotes,
        num_duplicate_quotes,
        num_missing_quotes,
        num_errors
    );

    Ok(())
}
