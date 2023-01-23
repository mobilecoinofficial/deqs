// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_quote_book::InMemoryQuoteBook;
use deqs_server::{Msg, Server, ServerConfig};
use libp2p::{
    futures::StreamExt,
    identity, ping,
    swarm::{SwarmBuilder, SwarmEvent},
    tokio_development_transport, Multiaddr, NetworkBehaviour, PeerId,
};
use libp2p_swarm::keep_alive;
use mc_common::logger::{log, o};
use mc_util_grpc::AdminServer;
use postage::{broadcast, prelude::Stream};
use std::sync::Arc;

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

#[derive(NetworkBehaviour, Default)]
struct Behaviour {
    keep_alive: keep_alive::Behaviour,
    ping: ping::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

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

    let behaviour = Behaviour::default();
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
    for peer in config.peers.iter() {
        let remote: Multiaddr = peer.parse()?;
        swarm.dial(remote)?;
        log::info!(&logger, "p2p: Dialed {}", peer);
    }
    tokio::spawn(async move {
        println!("in swarm loop");
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {:?}", address),
                SwarmEvent::Behaviour(event) => println!("{:?}", event),
                _ => {}
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
