// Copyright (c) 2023 MobileCoin Inc.

use super::{Behaviour, Error, OutEvent};
use libp2p::{
    futures::StreamExt,
    gossipsub::{GossipsubEvent, GossipsubMessage, IdentTopic},
    identify,
    kad::{record::Key, KademliaEvent, QueryResult},
    multiaddr::Protocol,
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use libp2p_swarm::NetworkBehaviour;
use mc_common::logger::{log, Logger};
use std::fmt::Debug;
use tokio::{
    sync::mpsc,
    time::{interval, Duration},
};

/// Key used to do peer discovery with Kademlia.
const KAD_PEER_KEY: &str = "mc/deqs/p2p/kad/peer";

/// Bootstrap interval. This is the interval at which we will attempt to
/// re-bootstrap. This is needed to reconnect our bootstrap peers
/// in case the connection is lost.
const BOOTSTRAP_INTERVAL: Duration = Duration::from_secs(10);

/// The `Network` is a convenience wrapper around the `Swarm` using the
/// `Behaviour` in this crate. It can be constructed with the `NetworkBuilder`
/// for convenience.
///
/// Communication between the `Network` and its client(s) happens only through
/// mpsc channels, and is fully async.
///
///
/// The `Network`:
/// - receives `Instructions` over the `instruction_rx` channel
/// - reports events over the `notification_tx` channel
pub struct Network {
    instruction_rx: mpsc::UnboundedReceiver<Instruction>,
    notification_tx: mpsc::UnboundedSender<Notification>,
    swarm: Swarm<Behaviour>,
    peer_discovery_key: Key,
    shutdown_tx: mpsc::UnboundedSender<()>,
    shutdown_rx: mpsc::UnboundedReceiver<()>,
    logger: Logger,
}

impl Network {
    pub fn new(
        instruction_rx: mpsc::UnboundedReceiver<Instruction>,
        notification_tx: mpsc::UnboundedSender<Notification>,
        swarm: Swarm<Behaviour>,
        logger: Logger,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel();

        Network {
            instruction_rx,
            notification_tx,
            swarm,
            peer_discovery_key: Key::new(&KAD_PEER_KEY),
            shutdown_tx,
            shutdown_rx,
            logger,
        }
    }

    pub fn shutdown(&mut self) {
        if self.shutdown_tx.send(()).is_err() {
            log::warn!(&self.logger, "shutdown already requested");
        }
    }

    pub async fn run(mut self, bootstrap_peers: &[Multiaddr]) -> Result<(), Error> {
        self.bootstrap(bootstrap_peers)?;

        let mut interval = interval(BOOTSTRAP_INTERVAL);

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => self.handle_swarm_event(event).await,
                Some(instruction) = self.instruction_rx.recv() =>  self.handle_instruction(instruction).await,
                _ = interval.tick() => {
                    log::trace!(&self.logger, "periodic interval!");
                    if let Err(err) = self.bootstrap(bootstrap_peers) {
                        log::warn!(&self.logger, "bootstrap failed: {:?}", err);
                    }
                }
                _ = self.shutdown_rx.recv() => {
                    log::info!(&self.logger, "shutdown requested");
                    break
                }
            }
        }

        Ok(())
    }

    pub fn peer_id(&self) -> &PeerId {
        self.swarm.local_peer_id()
    }

    async fn handle_swarm_event<TErr>(&mut self, event: SwarmEvent<OutEvent, TErr>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                log::info!(&self.logger, "Listening on {:?}", address)
            }
            SwarmEvent::Behaviour(OutEvent::Ping(ping)) => {
                log::info!(&self.logger, "ping {:?}", ping);
            }
            SwarmEvent::Behaviour(OutEvent::Kademlia(KademliaEvent::OutboundQueryCompleted {
                result,
                ..
            })) => match result {
                QueryResult::GetProviders(Ok(ok)) if ok.key == self.peer_discovery_key => {
                    for peer in ok.providers {
                        let addrs = self.swarm.behaviour_mut().kademlia.addresses_of_peer(&peer);
                        log::info!(
                            &self.logger,
                            "Peer {:?} provides key {:?} via addresses {:?}",
                            peer,
                            std::str::from_utf8(ok.key.as_ref()).unwrap(),
                            addrs,
                        );
                        for addr in addrs {
                            log::warn!(&self.logger, "dial failed: {:?}", self.swarm.dial(addr));
                        }
                    }
                }
                evt => {
                    log::info!(&self.logger, "Kademlia event: {:?}", evt);
                }
            },

            SwarmEvent::Behaviour(OutEvent::Gossipsub(GossipsubEvent::Message {
                message, ..
            })) => {
                log::info!(&self.logger, "Gossipsub message: {:?}", message);

                self.notify(Notification::GossipMessage(message));
            }

            SwarmEvent::Behaviour(OutEvent::Identify(e)) => {
                log::info!(&self.logger, "IdentifyEvent: {:?}", e);

                if let identify::Event::Received {
                    peer_id,
                    info:
                        identify::Info {
                            listen_addrs,
                            protocols,
                            ..
                        },
                } = e
                {
                    if protocols
                        .iter()
                        .any(|p| p.as_bytes() == libp2p::kad::protocol::DEFAULT_PROTO_NAME)
                    {
                        for addr in listen_addrs {
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, addr);
                        }
                    }
                }
            }

            SwarmEvent::Behaviour(event) => log::info!(&self.logger, "{:?}", event),
            _ => {}
        }
    }

    async fn handle_instruction(&mut self, instruction: Instruction) {
        log::debug!(self.logger, "handling instruction {:?}", instruction);

        match instruction {
            Instruction::PeerList => {
                self.notify(Notification::PeerList(
                    self.swarm
                        .behaviour()
                        .gossipsub
                        .all_peers()
                        .map(|peer| *peer.0)
                        .collect::<Vec<_>>(),
                ));
            }

            Instruction::PublishGossip { topic, message } => {
                log::info!(&self.logger, "publishing gossip {} {:?}", topic, message);
                if let Err(err) = self.swarm.behaviour_mut().gossipsub.publish(topic, message) {
                    self.notify(Notification::Err(err.into()));
                }
            }

            Instruction::SubscribeGossip { topic } => {
                log::info!(&self.logger, "subscribing to gossip {}", topic);
                if let Err(err) = self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    self.notify(Notification::Err(err.into()));
                }
            }
        }
    }

    fn notify(&mut self, notification: Notification) {
        if let Err(err) = self.notification_tx.send(notification) {
            log::warn!(&self.logger, "Failed to send notification: {:?}", err);
        }
    }

    fn bootstrap(&mut self, bootstrap_peers: &[Multiaddr]) -> Result<(), Error> {
        // First, we add the addresses of the bootstrap nodes to our view of the DHT
        for orig_peer_addr in bootstrap_peers.iter() {
            let mut peer_addr = orig_peer_addr.clone();
            match peer_addr.pop() {
                Some(Protocol::P2p(peer_id)) => {
                    let peer_id = PeerId::from_multihash(peer_id)
                        .map_err(|err| Error::Multihash(format!("{:?}", err)))?;

                    self.swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, peer_addr.clone());

                    // Connect to the peer
                    self.swarm.dial(peer_addr)?;
                    log::info!(&self.logger, "p2p: Dialed {}", orig_peer_addr);
                }
                other => {
                    return Err(Error::InvalidPeerAddress(
                        orig_peer_addr.clone(),
                        format!("{:?}", other),
                    ));
                }
            }
        }

        // Next, we add our own info to the DHT. This will then automatically be
        // shared with the other peers on the DHT. This operation will
        // fail if we are a bootstrap peer.
        if !bootstrap_peers.is_empty() {
            self.swarm
                .behaviour_mut()
                .kademlia
                .bootstrap()
                .map_err(|err| Error::Bootstrap(err.to_string()))?;
        }

        // Announce our discovery key, this lets other peers find us over the DHT
        self.swarm
            .behaviour_mut()
            .kademlia
            .start_providing(self.peer_discovery_key.clone())?;

        // Search the DHT for other nodes
        self.swarm
            .behaviour_mut()
            .kademlia
            .get_providers(self.peer_discovery_key.clone());

        Ok(())
    }
}

/// An instruction for the network to do something. Most likely to send a
/// message of some sort
#[derive(Debug, Clone)]
pub enum Instruction {
    /// Instruct the network to provide a list of all peers it is aware of
    PeerList,

    /// Send a gossip message
    PublishGossip {
        /// The topic to publish to
        topic: IdentTopic,

        /// The message to publish
        message: Vec<u8>,
    },

    /// Subscribe to a gossip topic
    SubscribeGossip {
        /// The topic to subscribe to
        topic: IdentTopic,
    },
}

/// A notification from the network to one of its consumers.
/// Either arbitrary data, the list of known peers or an error.
#[derive(Debug)]
pub enum Notification {
    PeerList(Vec<PeerId>),
    GossipMessage(GossipsubMessage),
    Err(Error),
}
