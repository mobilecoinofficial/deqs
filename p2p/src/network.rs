// Copyright (c) 2023 MobileCoin Inc.

use super::{Behaviour, Error, OutEvent, RpcRequest, RpcResponse};
use libp2p::{
    futures::StreamExt,
    gossipsub::{
        error::{PublishError, SubscriptionError},
        GossipsubEvent, GossipsubMessage, IdentTopic, MessageId,
    },
    identify,
    kad::{record::Key, KademliaEvent, QueryResult},
    multiaddr::Protocol,
    request_response::{RequestId, RequestResponseEvent, RequestResponseMessage, ResponseChannel},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use libp2p_swarm::NetworkBehaviour;
use mc_common::logger::{log, Logger};
use std::{collections::HashMap, error::Error as StdError, fmt::Debug};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver},
        oneshot,
    },
    time::{interval, Duration},
};

#[derive(Clone)]
pub struct Client<REQ: RpcRequest, RESP: RpcResponse> {
    sender: mpsc::UnboundedSender<Instruction<REQ, RESP>>,
}

impl<REQ: RpcRequest, RESP: RpcResponse> Client<REQ, RESP> {
    pub async fn rpc_request(
        &mut self,
        peer: PeerId,
        request: REQ,
    ) -> Result<RESP, Box<dyn StdError + Send>> {
        let (response_sender, response_receiver) = oneshot::channel();
        self.sender
            .send(Instruction::RpcRequest {
                peer,
                request,
                response_sender,
            })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }

    /// Respond with the provided file content to the given request.
    pub async fn rpc_response(&mut self, response: RESP, channel: ResponseChannel<RESP>) {
        self.sender
            .send(Instruction::RpcResponse { response, channel })
            .expect("Command receiver not to be dropped.");
    }

    pub async fn subscribe_gossip(&mut self, topic: IdentTopic) -> Result<bool, SubscriptionError> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender
            .send(Instruction::SubscribeGossip {
                topic,
                response_sender,
            })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }

    pub async fn publish_gossip(
        &mut self,
        topic: IdentTopic,
        msg: Vec<u8>,
    ) -> Result<MessageId, PublishError> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender
            .send(Instruction::PublishGossip {
                topic,
                msg,
                response_sender,
            })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }
}

/// Key used to do peer discovery with Kademlia.
const KAD_PEER_KEY: &str = "mc/deqs/p2p/kad/peer";

/// Bootstrap interval. This is the interval at which we will attempt to
/// re-bootstrap. This is needed to reconnect our bootstrap peers
/// in case the connection is lost.
const BOOTSTRAP_INTERVAL: Duration = Duration::from_secs(10);

// TODO
#[derive(Debug)]
pub enum Event<REQ: RpcRequest, RESP: RpcResponse> {
    /// Incoming RPC request
    RpcRequest {
        request: REQ,
        channel: ResponseChannel<RESP>,
    },

    /// Connection established with a peer.
    ConnectionEstablished { peer_id: PeerId },

    /// Gossip message received.
    GossipMessage { message: GossipsubMessage },
}

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
pub struct Network<REQ: RpcRequest, RESP: RpcResponse> {
    // instruction_rx: mpsc::UnboundedReceiver<Instruction>,
    // notification_tx: mpsc::UnboundedSender<Notification>,
    command_receiver: mpsc::UnboundedReceiver<Instruction<REQ, RESP>>,
    event_sender: mpsc::UnboundedSender<Event<REQ, RESP>>,
    swarm: Swarm<Behaviour<REQ, RESP>>,
    peer_discovery_key: Key,
    shutdown_tx: mpsc::UnboundedSender<()>,
    shutdown_rx: mpsc::UnboundedReceiver<()>,
    logger: Logger,

    pending_rpc_requests:
        HashMap<RequestId, oneshot::Sender<Result<RESP, Box<dyn StdError + Send>>>>,
}

impl<REQ: RpcRequest, RESP: RpcResponse> Network<REQ, RESP> {
    pub fn new(
        // instruction_rx: mpsc::UnboundedReceiver<Instruction>,
        // notification_tx: mpsc::UnboundedSender<Notification>,
        swarm: Swarm<Behaviour<REQ, RESP>>,
        logger: Logger,
    ) -> (Self, UnboundedReceiver<Event<REQ, RESP>>, Client<REQ, RESP>) {
        let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel();
        let (command_sender, command_receiver) = mpsc::unbounded_channel();
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        let client = Client {
            sender: command_sender,
        };

        let network = Network {
            // instruction_rx,
            // notification_tx,
            command_receiver,
            event_sender,
            swarm,
            peer_discovery_key: Key::new(&KAD_PEER_KEY),
            shutdown_tx,
            shutdown_rx,
            logger,
            pending_rpc_requests: HashMap::new(),
        };

        (network, event_receiver, client)
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
                Some(instruction) = self.command_receiver.recv() =>  self.handle_instruction(instruction).await,
                // Some(instruction) = self.instruction_rx.recv() =>  self.handle_instruction(instruction).await,
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

    async fn handle_swarm_event<TErr: Debug>(
        &mut self,
        event: SwarmEvent<OutEvent<REQ, RESP>, TErr>,
    ) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                log::info!(&self.logger, "Listening on {:?}", address)
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                concurrent_dial_errors: _,
            } => {
                // First connection to this peer
                if u32::from(num_established) == 1u32 {
                    log::info!(
                        &self.logger,
                        "Connection established: {:?} @ {:?}",
                        peer_id,
                        endpoint
                    );

                    self.event_sender
                        .send(Event::ConnectionEstablished { peer_id })
                        .unwrap();
                }
            }

            SwarmEvent::Behaviour(OutEvent::RequestResponse(RequestResponseEvent::Message {
                message,
                ..
            })) => match message {
                RequestResponseMessage::Request {
                    request, channel, ..
                } => {
                    self.event_sender
                        .send(Event::RpcRequest { request, channel })
                        .unwrap();
                }
                RequestResponseMessage::Response {
                    request_id,
                    response,
                } => {
                    let _ = self
                        .pending_rpc_requests
                        .remove(&request_id)
                        .expect("Request to still be pending.") // TODO
                        .send(Ok(response));
                }
            },

            SwarmEvent::Behaviour(OutEvent::RequestResponse(
                RequestResponseEvent::OutboundFailure {
                    request_id, error, ..
                },
            )) => {
                let _ = self
                    .pending_rpc_requests
                    .remove(&request_id)
                    .expect("Request to still be pending.")
                    .send(Err(Box::new(error)));
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
                        // for addr in addrs {
                        //     log::info!(&self.logger, "dial: {:?}",
                        // self.swarm.dial(addr)); }
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

                self.event_sender
                    .send(Event::GossipMessage { message })
                    .unwrap();
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
            event => {
                log::info!(&self.logger, "Other event: {:?}", event);
            }
        }
    }

    async fn handle_instruction(&mut self, instruction: Instruction<REQ, RESP>) {
        match instruction {
            Instruction::RpcRequest {
                peer,
                request,
                response_sender,
            } => {
                let request_id = self.swarm.behaviour_mut().rpc.send_request(&peer, request);
                self.pending_rpc_requests
                    .insert(request_id, response_sender);
            }

            Instruction::RpcResponse { response, channel } => {
                self.swarm
                    .behaviour_mut()
                    .rpc
                    .send_response(channel, response)
                    .expect("Connection to peer to be still open.");
            }

            Instruction::PublishGossip {
                topic,
                msg,
                response_sender,
            } => {
                let resp = self.swarm.behaviour_mut().gossipsub.publish(topic, msg);

                response_sender.send(resp).expect("Receiver to be open.");
            }

            Instruction::SubscribeGossip {
                topic,
                response_sender,
            } => {
                let resp = self.swarm.behaviour_mut().gossipsub.subscribe(&topic);

                response_sender.send(resp).expect("Receiver to be open.");
            }
        }
    }

    // async fn handle_instruction(&mut self, instruction: Instruction) {
    //     log::debug!(self.logger, "handling instruction {:?}", instruction);

    //     match instruction {
    //         Instruction::PeerList => {
    //             self.notify(Notification::PeerList(
    //                 self.swarm
    //                     .behaviour()
    //                     .gossipsub
    //                     .all_peers()
    //                     .map(|peer| *peer.0)
    //                     .collect::<Vec<_>>(),
    //             ));
    //         }

    //         Instruction::PublishGossip { topic, message } => {
    //             log::info!(&self.logger, "publishing gossip {} {:?}", topic,
    // message);             if let Err(err) =
    // self.swarm.behaviour_mut().gossipsub.publish(topic, message) {
    //                 self.notify(Notification::Err(err.into()));
    //             }
    //         }

    //         Instruction::SubscribeGossip { topic } => {
    //             log::info!(&self.logger, "subscribing to gossip {}", topic);
    //             if let Err(err) =
    // self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
    // self.notify(Notification::Err(err.into()));             }
    //         }
    //     }
    // }

    // fn notify(&mut self, notification: Notification) {
    //     if let Err(err) = self.notification_tx.send(notification) {
    //         log::warn!(&self.logger, "Failed to send notification: {:?}", err);
    //     }
    // }

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
#[derive(Debug)]
pub enum Instruction<REQ: RpcRequest, RESP: RpcResponse> {
    // /// Instruct the network to provide a list of all peers it is aware of
    // PeerList,
    /// Send a gossip message
    PublishGossip {
        /// The topic to publish to
        topic: IdentTopic,

        /// The message to publish
        msg: Vec<u8>,

        response_sender: oneshot::Sender<Result<MessageId, PublishError>>,
    },

    /// Subscribe to a gossip topic
    SubscribeGossip {
        /// The topic to subscribe to
        topic: IdentTopic,
        response_sender: oneshot::Sender<Result<bool, SubscriptionError>>,
    },

    RpcRequest {
        peer: PeerId,
        request: REQ,
        response_sender: oneshot::Sender<Result<RESP, Box<dyn StdError + Send>>>,
    },

    RpcResponse {
        response: RESP,
        channel: ResponseChannel<RESP>,
    },
}

// /// A notification from the network to one of its consumers.
// #[derive(Debug)]
// pub enum Notification {
//     PeerList(Vec<PeerId>),
//     GossipMessage(GossipsubMessage),
//     ConnectionEstablished(PeerId),
//     Err(Error),
// }
