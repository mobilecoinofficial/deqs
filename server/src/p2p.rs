// Copyright (c) 2023 MobileCoin Inc.

use crate::Error;
use deqs_p2p::{
    libp2p::{
        gossipsub::{GossipsubMessage, IdentTopic},
        identity::Keypair,
        Multiaddr,
    },
    Behaviour, Client, Network, NetworkBuilder, NetworkEvent, NetworkEventLoopHandle,
};
use deqs_quote_book::{Quote, QuoteBook};
use futures::executor::block_on;
use mc_common::logger::{log, Logger};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Request {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Response {}

pub struct P2P<QB: QuoteBook> {
    _quote_book: QB,
    _logger: Logger,
    client: Client<Request, Response>,
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

        Ok((
            Self {
                _quote_book: quote_book,
                _logger: logger,
                client,
                event_loop_handle,
                msg_bus_topic,
            },
            events,
        ))
    }

    pub async fn broadcast_sci_quote_added(&mut self, quote: &Quote) -> Result<(), Error> {
        let bytes = mc_util_serial::serialize(quote)?;
        let _message_id = self
            .client
            .publish_gossip(self.msg_bus_topic.clone(), bytes)
            .await?;
        Ok(())
    }

    pub async fn handle_gossip_message(&mut self, message: GossipsubMessage) -> Result<(), Error> {
        if message.topic == self.msg_bus_topic.hash() {
            self.handle_msg_bus_message(message.data).await?;
        } else {
            log::warn!(
                self._logger,
                "Received gossip message with unknown topic: {:?}",
                message.topic
            );
        }
        Ok(())
    }

    async fn handle_msg_bus_message(&mut self, bytes: Vec<u8>) -> Result<(), Error> {
        let quote: Quote = mc_util_serial::deserialize(&bytes)?;
        panic!("{:?}", quote);
        //Ok(())
    }
}

impl<QB: QuoteBook> Drop for P2P<QB> {
    fn drop(&mut self) {
        block_on(self.event_loop_handle.shutdown());
    }
}
