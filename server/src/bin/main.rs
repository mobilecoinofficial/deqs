// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_quote_book::InMemoryQuoteBook;
use deqs_server::{Msg, Server, ServerConfig};
use libp2p::{
    futures::StreamExt,
    identity,
    identify,
    kad::{
        record::{store::MemoryStore, Key},
        Kademlia, KademliaConfig, KademliaEvent, QueryResult,
    },
    mdns, ping,
    swarm::{SwarmBuilder, SwarmEvent},
    tokio_development_transport, Multiaddr, NetworkBehaviour, PeerId,
};
use libp2p_swarm::{keep_alive, NetworkBehaviour};
use mc_common::logger::{log, o};
use mc_util_grpc::AdminServer;
use postage::{broadcast, prelude::Stream};
use std::{str::FromStr, sync::Arc};
use tokio::{io::AsyncBufReadExt, time::Duration};
use void::Void;

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "OutEvent")]
struct Behaviour {
    keep_alive: keep_alive::Behaviour,
    ping: ping::Behaviour,
    mdns: mdns::TokioMdns,
    kademlia: Kademlia<MemoryStore>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum OutEvent {
    Mdns(mdns::MdnsEvent),
    Ping(ping::Event),
    Void(Void),
    Kademlia(KademliaEvent),
    Identify(identify::Event),
}
impl From<mdns::MdnsEvent> for OutEvent {
    fn from(event: mdns::MdnsEvent) -> Self {
        OutEvent::Mdns(event)
    }
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    let (msg_bus_tx, mut msg_bus_rx) = broadcast::channel::<Msg>(MSG_BUS_QUEUE_SIZE);
    let quote_book = InMemoryQuoteBook::default();

    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!(logger, "p2p: local peer id: {:?}", local_peer_id);
    //let transport = libp2p::development_transport(local_key).await?;
    // let transport = TokioTcpTransport::new(GenTcpConfig::default().nodelay(true))
    //     .upgrade(upgrade::Version::V1)
    //     .authenticate(
    //         noise::NoiseAuthenticated::xx(&local_key)
    //             .expect("Signing libp2p-noise static DH keypair failed."),
    //     )
    //     .multiplex(mplex::MplexConfig::new())
    //     .boxed();

    // Create a Kademlia behaviour.
    let mut cfg = KademliaConfig::default();
    cfg.set_query_timeout(Duration::from_secs(5 * 60));
    let store = MemoryStore::new(local_peer_id);
    let mut kademlia = Kademlia::with_config(local_peer_id, store, cfg);
    for (peer, peer_id) in config.peers.iter().zip(config.peer_ids.iter()) {
        let remote: Multiaddr = peer.parse()?;
        //swarm.dial(remote)?;
        kademlia.add_address(&PeerId::from_str(peer_id)?, remote);
        log::info!(&logger, "p2p: Dialed {}", peer);
    }
    let key = Key::new(&"keh");
    kademlia
        .start_providing(key.clone())
        .expect("Failed to start providing key");

    let behaviour = Behaviour {
        keep_alive: keep_alive::Behaviour::default(),
        ping: ping::Behaviour::default(),
        mdns: mdns::TokioMdns::new(mdns::MdnsConfig::default())?,
        kademlia,
    };
    let mut swarm = SwarmBuilder::new(
        tokio_development_transport(local_key)?,
        behaviour,
        local_peer_id,
    )
    .executor(Box::new(|fut| {
        tokio::spawn(fut);
    }))
    .build();
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
    let logger2 = logger.clone();
    tokio::spawn(async move {
        println!("in swarm loop");
        loop {
            tokio::select! {
                line = stdin.next_line() => {
                println!("reading done: {:?}", line);
                        swarm.behaviour_mut().kademlia.get_providers(key.clone());
                }
                sel = swarm.select_next_some() => match sel {
                    SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {:?}", address),
                    SwarmEvent::Behaviour(OutEvent::Mdns(mdns::MdnsEvent::Discovered(peers))) => {
                        for (peer, addr) in peers {
                            log::info!(&logger2, "discovered {peer} {addr}");
                            swarm.dial(addr).expect("dial failed");
                        }
                    }
                    SwarmEvent::Behaviour(OutEvent::Mdns(mdns::MdnsEvent::Expired(expired))) => {
                        for (peer, addr) in expired {
                            log::info!(&logger2, "expired {peer} {addr}");
                            let _ = swarm.disconnect_peer_id(peer); // this panics?
                        }
                    }
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

    let mut server = Server::new(
        msg_bus_tx,
        quote_book,
        config.client_listen_uri.clone(),
        logger.clone(),
    );
    server.start().expect("Failed starting client GRPC server");

    let config_json = serde_json::to_string(&config).expect("failed to serialize config to JSON");
    let get_config_json = Arc::new(move || Ok(config_json.clone()));
    let id = config.client_listen_uri.to_string();
    let _admin_server = config.admin_listen_uri.as_ref().map(|admin_listen_uri| {
        AdminServer::start(
            None,
            admin_listen_uri,
            "DEQS".into(),
            id,
            Some(get_config_json),
            logger,
        )
        .expect("Failed starting admin server")
    });

    // Keep the server alive by just reading messages from the message bus.
    // This allows us to ensure we always have at least 1 receiver on the message
    // bus, which will prevent sends from failing.
    loop {
        let _ = msg_bus_rx.recv().await;
    }
}
