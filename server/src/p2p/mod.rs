// Copyright (c) 2023 MobileCoin Inc.

//! Peer to peer networking.

mod behaviour;
mod error;

pub use behaviour::{Behaviour, OutEvent};
pub use error::Error;

use libp2p::{
    core::{muxing::StreamMuxerBox, transport, upgrade, upgrade::SelectUpgrade},
    dns,
    futures::StreamExt,
    gossipsub::IdentTopic,
    identify, identity,
    kad::{record::Key, KademliaEvent, QueryResult},
    mplex,
    multiaddr::Protocol,
    noise::{self},
    swarm::{AddressScore, SwarmBuilder, SwarmEvent},
    tcp, websocket, yamux, Multiaddr, PeerId, Transport,
};
use libp2p_swarm::{NetworkBehaviour, Swarm};
use mc_common::logger::{log, Logger};
use tokio::{io::AsyncBufReadExt, task::JoinHandle};

const BROADCAST_TOPIC: &str = "BROADCAST";

pub struct P2P {
    pub join_handle: JoinHandle<()>,
}

impl P2P {
    pub fn new(
        local_key: identity::Keypair,
        bootstrap_peers: &[Multiaddr],
        listen_addr: Multiaddr,
        external_addr: Option<&Multiaddr>,
        logger: Logger,
    ) -> Result<Self, Error> {
        let local_peer_id = PeerId::from(local_key.public());
        log::info!(logger, "p2p: local peer id: {:?}", local_peer_id);

        let behaviour = Behaviour::new(&local_key, local_peer_id)?;
        let key = Key::new(&"keh");

        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        ///////////////////////
        let topic = IdentTopic::new(BROADCAST_TOPIC);
        ////////////////////////////////

        let transport = Self::create_transport(&local_key)?;

        let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
            .executor(Box::new(|fut| {
                tokio::spawn(fut);
            }))
            .build();

        // First, we add the addresses of the bootstrap nodes to our view of the DHT
        for orig_peer_addr in bootstrap_peers.iter() {
            let mut peer_addr = orig_peer_addr.clone();
            match peer_addr.pop() {
                Some(Protocol::P2p(peer_id)) => {
                    let peer_id = PeerId::from_multihash(peer_id).expect("Can't parse peer id");
                    swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, peer_addr.clone());
                    //swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    swarm.dial(peer_addr)?;
                    log::info!(&logger, "p2p: Dialed {}", orig_peer_addr);
                }
                other => {
                    panic!("Invalid peer address {:?}, expected last component to be p2p multihash and instead got {:?}", orig_peer_addr, other);
                }
            }
        }

        // Next, we add our own info to the DHT. This will then automatically be
        // shared with the other peers on the DHT. This operation will
        // fail if we are a bootstrap peer.
        if !bootstrap_peers.is_empty() {
            swarm
                .behaviour_mut()
                .kademlia
                .bootstrap()
                .map_err(|err| Error::Bootstrap(err.to_string()))?;
        }

        swarm.listen_on(listen_addr)?;

        if let Some(addr) = external_addr {
            swarm.add_external_address(addr.clone(), AddressScore::Infinite);
        }

        swarm
            .behaviour_mut()
            .kademlia
            .start_providing(key.clone())
            .expect("Failed to start providing key");

        swarm.behaviour_mut().gossipsub.subscribe(&topic).unwrap();

        let join_handle = tokio::spawn(async move {
            println!("in swarm loop");
            loop {
                tokio::select! {
                    line = stdin.next_line() => {
                        println!("reading done: {:?}", line);
                        let line = line.unwrap().unwrap().trim().to_string();
                        match line.as_str() {
                            "prov" => {
                                swarm.behaviour_mut().kademlia.get_providers(key.clone());
                            }
                            "gos" => {
                                use rand::Rng;
                                let x = rand::thread_rng().gen_range(0..100000000);
                                if let Err(e) = swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .publish(topic.clone(), x.to_string().as_bytes())
                                {
                                    println!("Publish error: {:?}", e);
                                }
                            }
                            "b" => {
                                println!("{:?}", swarm.behaviour_mut().kademlia.bootstrap());
                            }
                            l => {
                                println!("???? {:?}", l);
                            }
                        }
                    }
                    sel = swarm.select_next_some() => Self::handle_swarm_event(&mut swarm, sel, &logger)
                }
            }
        });

        Ok(Self { join_handle })
    }

    fn handle_swarm_event<TErr>(
        swarm: &mut Swarm<Behaviour>,
        event: SwarmEvent<OutEvent, TErr>,
        logger: &Logger,
    ) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {:?}", address),
            SwarmEvent::Behaviour(OutEvent::Ping(ping)) => {
                log::info!(&logger, "ping {:?}", ping);
            }
            SwarmEvent::Behaviour(OutEvent::Kademlia(KademliaEvent::OutboundQueryCompleted {
                result,
                ..
            })) => match result {
                QueryResult::GetProviders(Ok(ok)) => {
                    for peer in ok.providers {
                        let addrs = (*swarm).behaviour_mut().kademlia.addresses_of_peer(&peer);
                        log::info!(
                            &logger,
                            "Peer {:?} provides key {:?}: {:?}",
                            peer,
                            std::str::from_utf8(ok.key.as_ref()).unwrap(),
                            addrs,
                        );
                        for addr in addrs {
                            log::crit!(&logger, "DIAL: {:?}", swarm.dial(addr));
                        }
                    }
                }
                evt => {
                    log::info!(&logger, "Kademlia event: {:?}", evt);
                }
            },

            SwarmEvent::Behaviour(OutEvent::Identify(e)) => {
                log::info!(&logger, "IdentifyEvent: {:?}", e);

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
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                        }
                    }
                }
            }

            SwarmEvent::Behaviour(event) => log::info!(&logger, "{:?}", event),
            _ => {}
        }
    }

    fn create_transport(
        local_key: &identity::Keypair,
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
        .authenticate(noise::NoiseAuthenticated::xx(&local_key)?)
        .multiplex(SelectUpgrade::new(
            yamux::YamuxConfig::default(),
            mplex::MplexConfig::default(),
        ))
        .timeout(std::time::Duration::from_secs(20))
        .boxed())
    }
}
