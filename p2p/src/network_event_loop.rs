// Copyright (c) 2023 MobileCoin Inc.

use super::{
    client::{Command},
    network::NetworkEvent,
    Behaviour, Error, OutEvent, RpcRequest, RpcResponse,
};
use libp2p::{
    futures::StreamExt,
    gossipsub::GossipsubEvent,
    identify,
    kad::{record::Key, KademliaEvent, QueryResult},
    multiaddr::Protocol,
    request_response::{RequestId, RequestResponseEvent, RequestResponseMessage},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};
use libp2p_swarm::NetworkBehaviour;
use mc_common::logger::{log, Logger};
use std::{collections::HashMap, error::Error as StdError, fmt::Debug};
use tokio::{
    sync::{
        mpsc,
        oneshot,
    },
    time::{interval, Duration},
};

/// Key used to do peer discovery with Kademlia.
const KAD_PEER_KEY: &str = "mc/deqs/p2p/kad/peer";

/// Bootstrap interval. This is the interval at which we will attempt to
/// re-bootstrap. This is needed to reconnect our bootstrap peers
/// in case the connection is lost.
const BOOTSTRAP_INTERVAL: Duration = Duration::from_secs(10);

type RpcRequestsMap<RESP> =
    HashMap<RequestId, oneshot::Sender<Result<RESP, Box<dyn StdError + Send>>>>;

/// TODO
pub struct NetworkEventLoop<REQ: RpcRequest, RESP: RpcResponse> {
    command_receiver: mpsc::UnboundedReceiver<Command<REQ, RESP>>,
    event_sender: mpsc::UnboundedSender<NetworkEvent<REQ, RESP>>,
    swarm: Swarm<Behaviour<REQ, RESP>>,
    peer_discovery_key: Key,
    shutdown_tx: mpsc::UnboundedSender<()>,
    shutdown_rx: mpsc::UnboundedReceiver<()>,
    logger: Logger,

    pending_rpc_requests: RpcRequestsMap<RESP>,
}

impl<REQ: RpcRequest, RESP: RpcResponse> NetworkEventLoop<REQ, RESP> {
    pub fn new(
        command_receiver: mpsc::UnboundedReceiver<Command<REQ, RESP>>,
        event_sender: mpsc::UnboundedSender<NetworkEvent<REQ, RESP>>,
        swarm: Swarm<Behaviour<REQ, RESP>>,
        logger: Logger,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel();

        Self {
            command_receiver,
            event_sender,
            swarm,
            peer_discovery_key: Key::new(&KAD_PEER_KEY),
            shutdown_tx,
            shutdown_rx,
            logger,
            pending_rpc_requests: HashMap::new(),
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
                Some(command) = self.command_receiver.recv() =>  self.handle_command(command).await,
                // Some(command) = self.command_rx.recv() =>  self.handle_command(command).await,
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
                        .send(NetworkEvent::ConnectionEstablished { peer_id })
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
                        .send(NetworkEvent::RpcRequest { request, channel })
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
                    .send(NetworkEvent::GossipMessage { message })
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

    async fn handle_command(&mut self, command: Command<REQ, RESP>) {
        match command {
            Command::PeerList { response_sender } => {
                let peer_ids = self
                    .swarm
                    .behaviour()
                    .gossipsub
                    .all_peers()
                    .map(|peer| *peer.0)
                    .collect::<Vec<_>>();

                response_sender
                    .send(peer_ids)
                    .expect("Receiver to be open.");
            }

            Command::RpcRequest {
                peer,
                request,
                response_sender,
            } => {
                let request_id = self.swarm.behaviour_mut().rpc.send_request(&peer, request);
                self.pending_rpc_requests
                    .insert(request_id, response_sender);
            }

            Command::RpcResponse { response, channel } => {
                self.swarm
                    .behaviour_mut()
                    .rpc
                    .send_response(channel, response)
                    .expect("Connection to peer to be still open.");
            }

            Command::PublishGossip {
                topic,
                msg,
                response_sender,
            } => {
                let resp = self.swarm.behaviour_mut().gossipsub.publish(topic, msg);

                response_sender.send(resp).expect("Receiver to be open.");
            }

            Command::SubscribeGossip {
                topic,
                response_sender,
            } => {
                let resp = self.swarm.behaviour_mut().gossipsub.subscribe(&topic);

                response_sender.send(resp).expect("Receiver to be open.");
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
