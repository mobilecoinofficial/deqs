// Copyright (c) 2023 MobileCoin Inc.

use super::{Behaviour, Error, OutEvent};
use libp2p::{
    futures::StreamExt,
    identify,
    kad::{KademliaEvent, QueryResult},
    multiaddr::Protocol,
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use libp2p_swarm::NetworkBehaviour;
use mc_common::logger::{log, Logger};
use std::fmt::Debug;
use tokio::sync::mpsc;

///
/// The `Network` is a convenience wrapper around the `Swarm` using the
/// `Behaviour` in this crate. It can be constructed with the `NetworkBuilder`
/// for convenience.
///
/// Communication between the `Network` and its client(s) happens only through
/// mpsc channels, and is fully async.
///
///
/// The `Network`:
/// - receives `Instructions`, which it passes on to the `InstructionHandler`
///   trait implemented on the swarm's NetworkBehaviour.
/// - receives events from the swarm and passes them on to the `EventHandler`
///   trait implemented on the swarm's `NetworkBehaviour`. The `EventHandler`
///   can notify the `Network`'s client by sending a `Notification`.
///
/// The `Network` does not know or care what the Instructions and Notifications
/// contain. It is up to the client(s) to give them meaning.
#[allow(dead_code)] // TODO
pub struct Network {
    instruction_rx: mpsc::UnboundedReceiver<Instruction>,
    notification_tx: mpsc::UnboundedSender<Notification>,
    swarm: Swarm<Behaviour>,
    logger: Logger,
}

impl Network {
    pub fn new(
        instruction_rx: mpsc::UnboundedReceiver<Instruction>,
        notification_tx: mpsc::UnboundedSender<Notification>,
        swarm: Swarm<Behaviour>,
        logger: Logger,
    ) -> Self {
        Network {
            instruction_rx,
            notification_tx,
            swarm,
            logger,
        }
    }

    pub async fn run(mut self, bootstrap_peers: &[Multiaddr]) -> Result<(), Error> {
        self.bootstrap(bootstrap_peers)?;

        loop {
            tokio::select! {
                //event = self.swarm.select_next_some() => self.swarm.behaviour_mut().handle_event(&self.notification_tx, event).await,
                event = self.swarm.select_next_some() => self.handle_swarm_event(event).await,
                Some(instruction) = self.instruction_rx.recv() =>  self.handle_instruction(instruction).await,
                else => {
                    log::warn!(&self.logger, "Both swarm and instruction receiver closed. Ending event loop");
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
                // TODO check key
                QueryResult::GetProviders(Ok(ok)) => {
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
        match instruction {
            Instruction::PeerList => {
                // Self::notify(
                //     notification_tx,
                //     Notification::PeerList(self.pubsub.all_peers().map(|peer|
                // *peer.0).collect()), );
                log::crit!(
                    &self.logger,
                    "peer list: {:?}",
                    self.swarm
                        .behaviour()
                        .gossipsub
                        .all_peers()
                        .map(|peer| *peer.0)
                        .collect::<Vec<_>>()
                );
            }
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

        Ok(())
    }
}

/// An instruction for the network to do something. Most likely to send a
/// message of some sort
#[derive(Debug, Clone)]
pub enum Instruction {
    /// Instruct the network to provide a list of all peers it is aware of
    PeerList,
}

/// A notification from the network to one of its consumers.
/// Either arbitrary data, the list of known peers or an error.
#[derive(Debug)]
pub enum Notification {
    PeerList(Vec<PeerId>),
    Err(Error),
}
