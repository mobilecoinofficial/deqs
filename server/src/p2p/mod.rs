// Copyright (c) 2023 MobileCoin Inc.

//! Peer to peer networking.

use libp2p::{
    core::{upgrade, upgrade::SelectUpgrade},
    dns,
    futures::StreamExt,
    gossipsub::{
        Gossipsub, GossipsubEvent, GossipsubMessage, IdentTopic as Topic, MessageAuthenticity,
        MessageId, ValidationMode,
    },
    identify, identity,
    kad::{
        record::{store::MemoryStore, Key},
        Kademlia, KademliaConfig, KademliaEvent, QueryResult,
    },
    mplex,
    multiaddr::Protocol,
    noise, ping,
    swarm::{AddressScore, SwarmBuilder, SwarmEvent},
    tcp, websocket, yamux, Multiaddr, NetworkBehaviour, PeerId, Transport,
};
use libp2p_swarm::{keep_alive, NetworkBehaviour};
use mc_common::logger::{log, Logger};
use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    iter,
};
use tokio::{io::AsyncBufReadExt, time::Duration};
use void::Void;

const BROADCAST_TOPIC: &str = "BROADCAST";
const KADEMLIA_PROTO_NAME: &[u8] = b"/mc/deqs/kad/1.0.0";
const IDENTIFY_PROTO_NAME: &str = "/mc/deqs/ide/1.0.0";
const GOSSIPSUB_PROTO_ID_PREFIX: &str = "mc/deqs/gossipsub";

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "OutEvent")]
struct Behaviour {
    keep_alive: keep_alive::Behaviour,
    //ping: ping::Behaviour,
    /// - `identify::Behaviour`: when peers connect and they both support this
    ///   protocol, they exchange `IdentityInfo`. We then uses this info to add
    ///   their listen addresses to the Kademlia DHT. Without
    ///   `identify::Behaviour`, the DHT would propagate the peer ids of peers,
    ///   but not their listen addresses, making it impossible for peers to
    ///   connect to them.
    identify: identify::Behaviour,

    // - `Kademlia`: this is the Distributed Hash Table implementation,
    ///   primarily used to distribute info about other peers on the
    ///   network this peer knows about to connected peers. This is a core
    ///   feature of `Kademlia` that triggers automatically.
    ///   This enables so-called DHT-routing, which enables peers to send
    ///   messages to peers they are not directly connected to.
    kademlia: Kademlia<MemoryStore>,

    /// - `Gossipsub`: this takes care of sending pubsub messages to all peers
    ///   this peer is aware of on a certain topic. Subscribe/unsubscribe
    ///   messages are also propagated. This works well in combination with
    ///   `identify::Behaviour` and `Kademlia`, because they ensure that
    ///   Gossipsub messages not only reach directly connected peers, but all
    ///   peers that can be reached through the DHT routing.
    gossipsub: Gossipsub,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum OutEvent {
    Ping(ping::Event),
    Void(Void),
    Kademlia(KademliaEvent),
    Identify(identify::Event),
    Gossipsub(GossipsubEvent),
}
impl From<ping::Event> for OutEvent {
    fn from(event: ping::Event) -> Self {
        OutEvent::Ping(event)
    }
}
impl From<Void> for OutEvent {
    fn from(event: Void) -> Self {
        OutEvent::Void(event)
    }
}
impl From<KademliaEvent> for OutEvent {
    fn from(event: KademliaEvent) -> Self {
        OutEvent::Kademlia(event)
    }
}
impl From<identify::Event> for OutEvent {
    fn from(event: identify::Event) -> Self {
        OutEvent::Identify(event)
    }
}
impl From<GossipsubEvent> for OutEvent {
    fn from(event: GossipsubEvent) -> Self {
        OutEvent::Gossipsub(event)
    }
}

pub struct P2P {}

impl P2P {
    pub fn new(
        local_key: identity::Keypair,
        bootstrap_peers: &[Multiaddr],
        listen_addr: Multiaddr,
        external_addr: Option<&Multiaddr>,
        logger: Logger,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let local_peer_id = PeerId::from(local_key.public());
        log::info!(logger, "p2p: local peer id: {:?}", local_peer_id);

        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        ///////////////////////
        let topic = Topic::new(BROADCAST_TOPIC);
        // To content-address message, we can take the hash of message and use it as an
        // ID.
        let message_id_fn = |message: &GossipsubMessage| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            MessageId::from(s.finish().to_string())
        };

        // Set a custom gossipsub
        let gossipsub_config = libp2p::gossipsub::GossipsubConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
            .validation_mode(ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message
            // signing)
            .message_id_fn(message_id_fn) // content-address messages. No two messages of the
            .protocol_id_prefix(GOSSIPSUB_PROTO_ID_PREFIX)
            // same content will be propagated.
            .build()
            .expect("Valid config");
        // build a gossipsub network behaviour
        let mut gossipsub: Gossipsub = Gossipsub::new(
            MessageAuthenticity::Signed(local_key.clone()),
            gossipsub_config,
        )
        .expect("Correct configuration");

        ////////////////////////////////

        // Create a Kademlia behaviour.
        let mut cfg = KademliaConfig::default();
        cfg.set_provider_publication_interval(Some(Duration::from_secs(10)));
        cfg.set_query_timeout(Duration::from_secs(5 * 60));
        cfg.set_protocol_names(iter::once(Cow::Borrowed(KADEMLIA_PROTO_NAME)).collect());
        //cfg.set_provider_publication_interval(Some(Duration::from_secs(1)));
        let store = MemoryStore::new(local_peer_id);
        let mut kademlia = Kademlia::with_config(local_peer_id, store, cfg);

        let key = Key::new(&"keh");
        kademlia
            .start_providing(key.clone())
            .expect("Failed to start providing key");

        // subscribes to our topic
        gossipsub.subscribe(&topic).unwrap();

        let identify = identify::Behaviour::new(identify::Config::new(
            IDENTIFY_PROTO_NAME.to_string(),
            local_key.public(),
        ));

        let behaviour = Behaviour {
            keep_alive: keep_alive::Behaviour::default(),
            //ping: ping::Behaviour::default(),
            //mdns: mdns::TokioMdns::new(mdns::MdnsConfig::default())?,
            kademlia,
            gossipsub,
            identify,
        };

        let transport = {
            let dns_tcp = dns::TokioDnsConfig::system(tcp::TokioTcpTransport::new(
                tcp::GenTcpConfig::new().nodelay(true),
            ))?;
            let ws_dns_tcp = websocket::WsConfig::new(dns::TokioDnsConfig::system(
                tcp::TokioTcpTransport::new(tcp::GenTcpConfig::new().nodelay(true)),
            )?);
            dns_tcp.or_transport(ws_dns_tcp)
        }
        .upgrade(upgrade::Version::V1)
        .authenticate(
            noise::NoiseAuthenticated::xx(&local_key)
                .expect("Signing libp2p-noise static DH keypair failed."),
        )
        .multiplex(SelectUpgrade::new(
            yamux::YamuxConfig::default(),
            mplex::MplexConfig::default(),
        ))
        .timeout(std::time::Duration::from_secs(20))
        .boxed();

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

        // // Next, we add our own info to the DHT. This will then automatically be
        // shared         // with the other peers on the DHT. This operation will
        // fail if we are a bootstrap peer.         kademlia
        //             .bootstrap()
        //             .map_err(|err| BootstrapError(err.to_string()))?;

        swarm.listen_on(listen_addr)?;

        if let Some(addr) = external_addr {
            swarm.add_external_address(addr.clone(), AddressScore::Infinite);
        }

        let logger2 = logger.clone();
        tokio::spawn(async move {
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
                    sel = swarm.select_next_some() => match sel {
                        SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {:?}", address),
                        SwarmEvent::Behaviour(OutEvent::Ping(ping)) => {
                            log::info!(&logger2, "ping {:?}", ping);
                        }
                        SwarmEvent::Behaviour(OutEvent::Kademlia(
                            KademliaEvent::OutboundQueryCompleted { result, .. },
                        )) => match result {
                            QueryResult::GetProviders(Ok(ok)) => {
                                for peer in ok.providers {
                                    let addrs = swarm.behaviour_mut().kademlia.addresses_of_peer(&peer);
                                    log::info!(
                                        &logger2,
                                        "Peer {:?} provides key {:?}: {:?}",
                                        peer,
                                        std::str::from_utf8(ok.key.as_ref()).unwrap(),
                                        addrs,
                                    );
                                    for addr in addrs {
                                        log::crit!(&logger2, "DIAL: {:?}", swarm.dial(addr));
                                    }
                                }
                            }
                            evt => {
                                log::info!(&logger2, "Kademlia event: {:?}", evt);
                            }
                        },

                        SwarmEvent::Behaviour(OutEvent::Identify(e)) => {
                            log::info!(&logger2, "IdentifyEvent: {:?}", e);

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
                                        swarm
                                            .behaviour_mut()
                                            .kademlia
                                            .add_address(&peer_id, addr);
                                    }
                                }
                            }
                        }

                        SwarmEvent::Behaviour(event) => log::info!(&logger2, "{:?}", event),
                        _ => {}
                    }
                }
            }
        });

        Ok(Self {})
    }
}
