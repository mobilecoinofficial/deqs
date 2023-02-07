// Copyright (c) 2023 MobileCoin Inc.

mod rpc;
mod rpc_error;

pub use rpc_error::{RpcError, RpcQuoteBookError};

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
use futures::{stream, StreamExt};
use mc_common::logger::{log, Logger};
use rpc::{Request, Response, RpcClient};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tokio::sync::mpsc::UnboundedReceiver;

/// Gossip topic for message-bus traffic.
const MSG_BUS_TOPIC: &str = "mc/deqs/server/msg-bus";

/// Maximum number of concurrent requests we will issue when syncing quotes.
const PEER_SYNC_MAX_CONCURRENT_REQUESTS: usize = 10;

/// Data type for messages sent over the message-bus gossip topic.
#[derive(Debug, Deserialize, Serialize)]
enum GossipMsgBusData {
    SciQuoteAdded(Quote),
    SciQuoteRemoved(QuoteId),
}

/// An object for containing the logic for interfacing with the P2P network.
pub struct P2P<QB: QuoteBook> {
    /// Quote book.
    quote_book: QB,

    /// Logger.
    logger: Logger,

    /// P2P network client
    client: Client<Request, Response>,

    /// Our custom behaviour RPC client (that operates on top of the p2p network
    /// client)
    rpc: RpcClient,

    /// P2P network event loop handle.
    /// We must keep the handle alive by holding it, otherwise the event loop
    /// will stop when it gets dropped.
    _event_loop_handle: NetworkEventLoopHandle,

    /// The gossip topic we use for exchanging message-bus messages.
    msg_bus_topic: IdentTopic,
}

impl<QB: QuoteBook> P2P<QB> {
    /// Construct a new P2P object. This returns both the object (that holds the
    /// majority of the deqs-server-specific p2p logic) and the network
    /// event receiver, which hands out asynchronous events from the p2p
    /// network. It is up to to the caller to read from it and feed the
    /// events to the P2P object's handle_network_event method.
    /// The caller is expected to have an event loop, so this can easily be
    /// added to it and saves us from having to manage a task inside this
    /// object.
    pub async fn new(
        quote_book: QB,
        bootstrap_peers: Vec<Multiaddr>,
        listen_addr: Option<Multiaddr>,
        external_addr: Option<Multiaddr>,
        keypair: Option<Keypair>,
        logger: Logger,
    ) -> Result<(Self, UnboundedReceiver<NetworkEvent<Request, Response>>), Error> {
        let msg_bus_topic = IdentTopic::new(MSG_BUS_TOPIC);

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
                _event_loop_handle: event_loop_handle,
                msg_bus_topic,
            },
            events,
        ))
    }

    /// Broadcast to other peers that a new quote has been added to the quote
    /// book.
    pub async fn broadcast_sci_quote_added(&mut self, quote: Quote) -> Result<(), Error> {
        let bytes = mc_util_serial::serialize(&GossipMsgBusData::SciQuoteAdded(quote))?;
        let _message_id = self
            .client
            .publish_gossip(self.msg_bus_topic.clone(), bytes)
            .await?;
        Ok(())
    }

    /// Broadcast to toher peers that a quote has been removed from the quote
    /// book.
    pub async fn broadcast_sci_quote_removed(&mut self, quote_id: QuoteId) -> Result<(), Error> {
        let bytes = mc_util_serial::serialize(&GossipMsgBusData::SciQuoteRemoved(quote_id))?;
        let _message_id = self
            .client
            .publish_gossip(self.msg_bus_topic.clone(), bytes)
            .await?;
        Ok(())
    }

    /// Handle an asynchronous event from the p2p network.
    pub async fn handle_network_event(&mut self, event: NetworkEvent<Request, Response>) {
        match event {
            NetworkEvent::ConnectionEstablished { peer_id } => {
                if let Err(err) = self.handle_connection_established(peer_id).await {
                    log::error!(
                        self.logger,
                        "handle_connection_established failed: {:?}",
                        err
                    )
                }
            }

            NetworkEvent::GossipMessage { message } => {
                if let Err(err) = self.handle_gossip_message(message).await {
                    log::error!(self.logger, "handle_gossip_message failed: {:?}", err)
                }
            }

            NetworkEvent::RpcRequest {
                peer,
                request,
                channel,
            } => {
                if let Err(err) = self.handle_rpc_request(peer, request, channel).await {
                    log::error!(self.logger, "handle_rpc_request failed: {:?}", err)
                }
            }

            event => {
                log::debug!(self.logger, "p2p event: {:?}", event);
            }
        }
    }

    /// Async network event: connection established with a peer.
    async fn handle_connection_established(&mut self, peer_id: PeerId) -> Result<(), Error> {
        log::info!(self.logger, "Connection established with peer: {}", peer_id);

        // Start a task to sync quotes from the peer.
        let rpc = self.rpc.clone();
        let quote_book = self.quote_book.clone();
        let logger = self.logger.clone();
        tokio::spawn(async move {
            if let Err(err) = sync_quotes_from_peer(peer_id, rpc, &quote_book, &logger).await {
                log::warn!(logger, "Failed to sync quotes from peer: {}", err);
            }
        });

        Ok(())
    }

    /// Async network event: incoming RPC request
    async fn handle_rpc_request(
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
                .unwrap_or_else(|err| Response::Error(RpcError::QuoteBook(err.into()))),

            Request::GetQuoteById(quote_id) => self
                .quote_book
                .get_quote_by_id(&quote_id)
                .map(Response::MaybeQuote)
                .unwrap_or_else(|err| Response::Error(RpcError::QuoteBook(err.into()))),
        };

        self.client.rpc_response(response, channel).await?;

        Ok(())
    }

    /// Async network event: incoming gossip message
    async fn handle_gossip_message(&mut self, message: GossipsubMessage) -> Result<(), Error> {
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

    /// Handle a gossip message on the message bus topic.
    async fn handle_msg_bus_message(&mut self, msg: GossipMsgBusData) -> Result<(), Error> {
        match msg {
            GossipMsgBusData::SciQuoteAdded(quote) => self.handle_sci_quote_added(quote).await,

            GossipMsgBusData::SciQuoteRemoved(quote_id) => {
                self.handle_sci_quote_removed(&quote_id).await
            }
        }
    }

    /// Handle a message on the message bus topic: a new quote has beeen added
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

    /// Handle a message on the message bus topic: a quote has been removed
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

// TODO we might want to have an always-running task that syncs quotes from
// peers sequentially, rather than spawning a new task for each peer.
// What happens right now is that we connect to multiple peers at the same time
// and end up syncing a lot of identical quotes from each of the peers.
async fn sync_quotes_from_peer(
    peer_id: PeerId,
    mut rpc: RpcClient,
    quote_book: &impl QuoteBook,
    logger: &Logger,
) -> Result<(), Error> {
    // Get all quote ids from peer.
    let remote_quote_ids = BTreeSet::from_iter(rpc.get_all_quote_ids(peer_id).await?);

    // Get all local quote ids, and find which ones we are missing.
    let local_quote_ids = BTreeSet::from_iter(quote_book.get_quote_ids(None)?);
    let missing_quote_ids: Vec<_> = remote_quote_ids
        .difference(&local_quote_ids)
        .cloned()
        .collect();
    let num_missing_quote_ids = missing_quote_ids.len();

    log::info!(
        logger,
        "Received {} quote ids from peer {:?}. Will need to sync {} quotes",
        remote_quote_ids.len(),
        peer_id,
        num_missing_quote_ids,
    );

    let start_time = tokio::time::Instant::now();

    // Iterate over each missing quote id, and request it from the peer.
    // We will be executing this in parallel (up to
    // PEER_SYNC_MAX_CONCURRENT_REQUESTS)
    let results = stream::iter(missing_quote_ids)
        .map(|quote_id| {
            let mut rpc = rpc.clone();
            let quote_id = quote_id.clone();
            let quote_book = quote_book.clone();
            let logger = logger.clone();

            async move {
                // Make an RPC call to the peer to get the quote.
                let quote = match rpc.get_quote_by_id(peer_id, quote_id).await {
                    Ok(Some(quote)) => quote,
                    Ok(None) => {
                        log::debug!(
                            logger,
                            "Peer {:?} did not have quote {}",
                            peer_id,
                            quote_id,
                        );
                        return Err(Error::QuoteBook(QuoteBookError::QuoteNotFound));
                    }
                    Err(err) => {
                        log::warn!(
                            logger,
                            "Failed to get quote {} from peer {:?}: {:?}",
                            quote_id,
                            peer_id,
                            err,
                        );
                        return Err(err);
                    }
                };

                // Add the quote to our local quote book.
                match quote_book.add_sci(quote.sci().clone(), Some(quote.timestamp())) {
                    Ok(quote) => {
                        log::debug!(
                            logger,
                            "Synced quote {} from peer {:?}",
                            quote.id(),
                            peer_id
                        );
                        Ok(())
                    }
                    Err(err @ QuoteBookError::QuoteAlreadyExists) => {
                        log::debug!(
                            logger,
                            "Failed to add quote {} from peer {:?}: Already exists (this is acceptable)",
                            quote.id(),
                            peer_id,
                        );
                        Err(err.into())
                    }
                    Err(err)=> {
                        log::warn!(
                            logger,
                            "Failed to add quote {} from peer {:?}: {:?}",
                            quote.id(),
                            peer_id,
                            err,
                        );
                        Err(err.into())
                    }
                }
            }
        })
        .buffered(PEER_SYNC_MAX_CONCURRENT_REQUESTS)
        .collect::<Vec<_>>()
        .await;

    let mut num_added_quotes = 0;
    let mut num_duplicate_quotes = 0;
    let mut num_errors = 0;
    let mut num_missing_quotes = 0;

    for result in results {
        match result {
            Ok(_) => num_added_quotes += 1,
            Err(Error::QuoteBook(QuoteBookError::QuoteAlreadyExists)) => num_duplicate_quotes += 1,
            Err(Error::QuoteBook(QuoteBookError::QuoteNotFound)) => num_missing_quotes += 1,
            Err(err) => {
                log::warn!(logger, "Failed to sync quote: {}", err);
                num_errors += 1
            }
        }
    }

    let duration = tokio::time::Instant::now() - start_time;

    log::info!(
        logger,
        "Queried {} quotes from peer {:?}: {} added, {} duplicates, {} missing, {} errors (took {} milliseconds)",
        num_missing_quote_ids,
        peer_id,
        num_added_quotes,
        num_duplicate_quotes,
        num_missing_quotes,
        num_errors,
        duration.as_millis(),
    );
    /*
    let mut num_added_quotes = 0;
    let mut num_duplicate_quotes = 0;
    let mut num_errors = 0;
    let mut num_missing_quotes = 0;

    for (i, quote_id) in missing_quote_ids.into_iter().enumerate() {
        log::debug!(
            logger,
            "Syncing quote {}/{} from peer {:?}: {}",
            i + 1,
            num_missing_quote_ids,
            peer_id,
            quote_id,
        );

        match rpc.get_quote_by_id(peer_id, quote_id).await {
            Ok(Some(quote)) => {
                match quote_book.add_sci(quote.sci().clone(), Some(quote.timestamp())) {
                    Ok(_) => {
                        log::debug!(
                            logger,
                            "Synced quote {} ({}/{}) from peer {:?}",
                            quote_id,
                            i + 1,
                            num_missing_quote_ids,
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
                            num_missing_quote_ids,
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
                            num_missing_quote_ids,
                            peer_id,
                            err,
                        );

                        num_errors += 1;
                    }
                }
            }

            Ok(None) => {
                log::debug!(logger, "Peer {:?} did not have quote {}", peer_id, quote_id);
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
    let end_time = tokio::time::Instant::now() - start_time;

    log::info!(
        logger,
        "Synced {} quotes from peer {:?}: {} added, {} duplicates, {} missing, {} errors (took {} milliseconds)",
        num_missing_quote_ids,
        peer_id,
        num_added_quotes,
        num_duplicate_quotes,
        num_missing_quotes,
        num_errors,
        end_time.as_millis(),
    );*/

    Ok(())
}
