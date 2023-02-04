// Copyright (c) 2023 MobileCoin Inc.

use deqs_p2p::{
    libp2p::{identity::Keypair, Multiaddr},
    Behaviour, Client, Network, NetworkBuilder, NetworkEvent, NetworkEventLoopHandle,
};
use deqs_quote_book::QuoteBook;
use futures::executor::block_on;
use mc_common::logger::Logger;
use tokio::sync::mpsc::UnboundedReceiver;

pub struct P2P<QB: QuoteBook> {
    _quote_book: QB,
    _logger: Logger,
    _client: Client<String, String>,
    event_loop_handle: NetworkEventLoopHandle,
}

impl<QB: QuoteBook> P2P<QB> {
    pub fn new(
        quote_book: QB,
        bootstrap_peers: Vec<Multiaddr>,
        listen_addr: Option<Multiaddr>,
        external_addr: Option<Multiaddr>,
        keypair: Option<Keypair>,
        logger: Logger,
    ) -> Result<(Self, UnboundedReceiver<NetworkEvent<String, String>>), deqs_p2p::Error> {
        let keypair = keypair.unwrap_or_else(Keypair::generate_ed25519);

        let behaviour = Behaviour::<String, String>::new(&keypair)?;
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
            client,
        } = network_builder.build()?;

        Ok((
            Self {
                _quote_book: quote_book,
                _logger: logger,
                _client: client,
                event_loop_handle,
            },
            events,
        ))
    }
}

impl<QB: QuoteBook> Drop for P2P<QB> {
    fn drop(&mut self) {
        block_on(self.event_loop_handle.shutdown());
    }
}
