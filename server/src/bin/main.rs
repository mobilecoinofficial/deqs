// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_quote_book::InMemoryQuoteBook;
use deqs_server::{Msg, Server, ServerConfig};
use libp2p::identity;
use mc_common::logger::o;
use mc_util_grpc::AdminServer;
use postage::{broadcast, prelude::Stream};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    let (msg_bus_tx, mut msg_bus_rx) = broadcast::channel::<Msg>(MSG_BUS_QUEUE_SIZE);
    let quote_book = InMemoryQuoteBook::default();

    let local_key = match config.p2p_keypair_path {
        Some(ref path) => {
            let bytes = std::fs::read(path)?;
            identity::Keypair::from_protobuf_encoding(&bytes)?
        }
        None => identity::Keypair::generate_ed25519(),
    };

    // let _p2p = P2P::new(
    //     local_key,
    //     &config.p2p_bootstrap_peers,
    //     config
    //         .p2p_listen
    //         .clone()
    //         .unwrap_or_else(|| "/ip4/0.0.0.0/tcp/0".parse().unwrap()),
    //     config.p2p_external_address.as_ref(),
    //     logger.clone(),
    // )
    // .unwrap();

    let (notification_tx, mut notification_rx) = mpsc::unbounded_channel();
    let (instruction_tx, instruction_rx) = mpsc::unbounded_channel();
    let behaviour = deqs_p2p::Behaviour::new(&local_key)?;
    let mut network_builder = deqs_p2p::NetworkBuilder::new(
        local_key,
        instruction_rx,
        notification_tx,
        behaviour,
        logger.clone(),
    )?;
    if let Some(ref listen_addr) = config.p2p_listen {
        network_builder = network_builder.listen_address(listen_addr.clone());
    }
    if let Some(ref external_addr) = config.p2p_external_address {
        network_builder = network_builder.external_addresses(vec![external_addr.clone()]);
    }
    let network = network_builder.build()?;
    let p2p_bootstrap_peers = config.p2p_bootstrap_peers.clone();
    tokio::spawn(async move {
        network
            .run(&p2p_bootstrap_peers)
            .await
            .expect("network run")
    });

    instruction_tx
        .send(deqs_p2p::Instruction::SubscribeGossip {
            topic: libp2p::gossipsub::IdentTopic::new("gos"),
        })
        .unwrap();

    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        loop {
            let line = stdin.next_line().await.unwrap().unwrap();
            match line.as_str() {
                "peers" => {
                    instruction_tx
                        .send(deqs_p2p::Instruction::PeerList)
                        .unwrap();
                }
                "gos" => {
                    let topic = libp2p::gossipsub::IdentTopic::new("gos");
                    use rand::Rng;
                    let x = rand::thread_rng().gen_range(0..100000000);
                    let message = x.to_string().as_bytes().to_vec();

                    instruction_tx
                        .send(deqs_p2p::Instruction::PublishGossip { topic, message })
                        .unwrap();
                }
                line => {
                    println!("Unknown command: {}", line);
                }
            }
        }
    });

    tokio::spawn(async move {
        loop {
            let msg = notification_rx.recv().await.unwrap();
            println!("Got message: {:?}", msg);
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
