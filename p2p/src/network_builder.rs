// Copyright (c) 2023 MobileCoin Inc.

use crate::Event;

use super::{Behaviour, Client, Error, Network, RpcRequest, RpcResponse};
use libp2p::{
    core::{muxing::StreamMuxerBox, transport, transport::upgrade, upgrade::SelectUpgrade},
    dns,
    identity::Keypair,
    mplex, noise,
    swarm::SwarmBuilder,
    tcp, websocket, yamux, Multiaddr, PeerId, Transport,
};
use mc_common::logger::{log, Logger};
use tokio::sync::mpsc::UnboundedReceiver;

pub struct NetworkBuilder<REQ: RpcRequest, RESP: RpcResponse> {
    keypair: Keypair,
    // instruction_rx: mpsc::UnboundedReceiver<Instruction>,
    // notification_tx: mpsc::UnboundedSender<Notification>,
    transport: transport::Boxed<(PeerId, StreamMuxerBox)>,
    listen_address: Multiaddr,
    external_addresses: Vec<Multiaddr>,
    behaviour: Behaviour<REQ, RESP>,
    logger: Logger,
}

impl<REQ: RpcRequest, RESP: RpcResponse> NetworkBuilder<REQ, RESP> {
    pub fn new(
        keypair: Keypair,
        // instruction_rx: mpsc::UnboundedReceiver<Instruction>,
        // notification_tx: mpsc::UnboundedSender<Notification>,
        behaviour: Behaviour<REQ, RESP>,
        logger: Logger,
    ) -> Result<Self, Error> {
        Ok(Self {
            transport: Self::default_transport(&keypair)?,
            keypair,
            // instruction_rx,
            // notification_tx,
            listen_address: Self::default_listen_address()?,
            external_addresses: vec![],
            behaviour,
            logger,
        })
    }

    pub fn listen_address(mut self, address: Multiaddr) -> Self {
        self.listen_address = address;
        self
    }

    pub fn external_addresses(mut self, addresses: Vec<Multiaddr>) -> Self {
        self.external_addresses = addresses;
        self
    }

    pub fn transport(mut self, transport: transport::Boxed<(PeerId, StreamMuxerBox)>) -> Self {
        self.transport = transport;
        self
    }

    /// Listen on all interfaces, on a random port
    fn default_listen_address() -> Result<Multiaddr, Error> {
        Ok("/ip4/127.0.0.1/tcp/0".parse()?)
    }

    fn default_transport(
        keypair: &Keypair,
    ) -> Result<transport::Boxed<(PeerId, StreamMuxerBox)>, Error> {
        Ok({
            let dns_tcp = dns::TokioDnsConfig::system(tcp::TokioTcpTransport::new(
                tcp::GenTcpConfig::new().nodelay(true),
            ))?;
            let ws_dns_tcp = websocket::WsConfig::new(dns::TokioDnsConfig::system(
                tcp::TokioTcpTransport::new(tcp::GenTcpConfig::new().nodelay(true)),
            )?);
            dns_tcp.or_transport(ws_dns_tcp)
        }
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::NoiseAuthenticated::xx(keypair)?)
        .multiplex(SelectUpgrade::new(
            yamux::YamuxConfig::default(),
            mplex::MplexConfig::default(),
        ))
        .timeout(std::time::Duration::from_secs(20))
        .boxed())
    }

    pub fn build(
        self,
    ) -> Result<
        (
            Network<REQ, RESP>,
            UnboundedReceiver<Event<REQ, RESP>>,
            Client<REQ, RESP>,
        ),
        Error,
    > {
        let mut swarm = SwarmBuilder::new(
            self.transport,
            self.behaviour,
            PeerId::from(self.keypair.public()),
        )
        .executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .build();

        swarm.listen_on(self.listen_address)?;
        for addr in self.external_addresses {
            swarm.add_external_address(addr, libp2p_swarm::AddressScore::Infinite);
        }

        log::info!(&self.logger, "Local PeerId: {}", swarm.local_peer_id());

        Ok(Network::new(
            // self.instruction_rx,
            // self.notification_tx,
            swarm,
            self.logger,
        ))
    }
}
