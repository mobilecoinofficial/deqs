// Copyright (c) 2023 MobileCoin Inc.

//! A simple p2p chat application that demonstrates the basic usage of the
//! deqs-p2p crate.
//!
//! To run it, start the first node with:
//! `RUST_LOG=chat cargo run --example chat -- \
//!   --seed 1 --listen-addr /ip4/127.0.0.1/tcp/5000`
//!
//! Using --seed 1 will generate a deterministic peer ID. Start more nodes with:
//! `RUST_LOG=chat ./target/debug/examples/chat \
//!   --bootstrap-peer
//! /ip4/127.0.0.1/tcp/5000/p2p/
//! 12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X`
//!
//! After seeing ConnectionEstablished in the logs you can start chatting by
//! typing `gossip <text>`. This will send a broadcast message over the gossip
//! network and you should see it on other nodes. To send a direct message to a
//! node via the RPC mechanism, type `direct <peer_id> <text>`. For example:
//! `direct 12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X hello`

use clap::Parser;
use deqs_p2p::{Network, NetworkBuilder, NetworkEvent};
use libp2p::{
    gossipsub::IdentTopic,
    identity::{self, ed25519},
    Multiaddr,
};
use mc_common::logger::{log, o};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Command-line configuration options for the chat application
#[derive(Parser)]
pub struct Config {
    /// Optional listening address (a random port will be chosen if not
    /// provided).
    #[clap(long)]
    listen_addr: Option<Multiaddr>,

    /// Optional seed used for generating a deterministic peer ID.
    #[clap(long)]
    seed: Option<u8>,

    /// Bootstrap p2p peers. Need to include a `/p2p/<hash>` postfix, e.g.
    /// `/ip4/127.0.0.1/tcp/49946/p2p/
    /// 12D3KooWDExx59EUZCN3kBJXKNHHmfWb1HShvMmzGxGWWpeWXHEp`
    #[clap(long = "bootstrap-peer")]
    pub bootstrap_peers: Vec<Multiaddr>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse();
    let (logger, global_logger_guard) = mc_common::logger::create_app_logger(o!());
    global_logger_guard.cancel_reset();

    // Create a public/private key pair, either random or based on a seed.
    let local_key = match config.seed {
        Some(seed) => {
            let mut bytes = [0u8; 32];
            bytes[0] = seed;
            let secret_key = ed25519::SecretKey::from_bytes(&mut bytes).expect(
                "this returns `Err` only if the length is wrong; the length is correct; qed",
            );
            identity::Keypair::Ed25519(secret_key.into())
        }
        None => identity::Keypair::generate_ed25519(),
    };
    let peer_id = local_key.public().to_peer_id();

    log::info!(logger, "Local peer ID: {}", peer_id);

    // Create the network.
    // Note that the Behaviour<> object is generic of two types: REQ and RESP.
    // These types represent the request and response types for RPC calls.
    // For this example, we use the String type for both, so that clients can send
    // each other text. In a real application, you would define your own request
    // and response types.
    let behaviour = deqs_p2p::Behaviour::<String, String>::new(&local_key)?;
    let mut network_builder = NetworkBuilder::new(
        local_key,
        behaviour,
        config.bootstrap_peers.clone(),
        logger.clone(),
    )?;
    if let Some(ref listen_addr) = config.listen_addr {
        network_builder = network_builder.listen_address(listen_addr.clone());
    }
    let Network {
        mut event_loop_handle,
        mut events,
        client,
    } = network_builder.build()?;

    // Subscribe to a gossip topic for receiving broadcast messages.
    let topic = IdentTopic::new("chat-gossip");
    client.subscribe_gossip(topic.clone()).await?;

    // Process events from the network and from the standard input.
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();

    loop {
        tokio::select! {
            // P2P network event
            event = events.recv() => {
                match event {
                    Some(NetworkEvent::NewListenAddr { address }) => {
                        log::info!(logger, "Listening on {}/p2p/{}", address, peer_id);
                    }

                    Some(NetworkEvent::GossipMessage { message }) => {
                        log::info!(logger, "Received gossip from {:?}: {:?}", message.source, String::from_utf8(message.data));
                    }

                    Some(NetworkEvent::RpcRequest { peer, request, channel }) => {
                        log::info!(logger, "Received RPC request from {:?}: {:?}", peer, request);
                        log::info!(logger, "Responding to RPC request: {:?}", client.rpc_response("ACK".to_string(), channel).await);
                    }

                    other => {
                        log::info!(logger, "Other network event: {:?}", other);
                    }
                }
            }

            // Stdin line
            line = stdin.next_line() => {

                match line {
                    Ok(Some(line)) => {
                        let (cmd, args) = split_into_two(&line);

                        match cmd {
                            "gossip" => {
                                // Broadcast the message to all peers.
                                log::info!(logger, "Publish gossip: {:?}", client.publish_gossip(topic.clone(), args.as_bytes().to_vec()).await);
                            }

                            "direct" => {
                                // Send the message to a specific peer.
                                let (peer_id, message) = split_into_two(args);
                                let peer_id = if let Ok(peer_id) = peer_id.parse() {
                                    peer_id
                                } else {
                                    log::error!(logger, "Invalid peer ID: {}", peer_id);
                                    continue;
                                };

                                log::info!(logger, "Send direct via RPC: {:?}", client.rpc_request(peer_id, message.to_string()).await);
                            }

                            "exit" => {
                                break;
                            }

                            other => {
                                log::error!(logger, "Unknown command: {}", other);
                            }
                        }
                    }

                    Ok(None) => {
                        // EOF
                        break;
                    }

                    Err(err) => {
                        log::error!(logger, "Error reading stdin: {}", err);
                    }
                }
            }

        }
    }

    // Gracefully shutdown the network.
    event_loop_handle.shutdown().await;

    Ok(())
}

fn split_into_two(line: &str) -> (&str, &str) {
    let mut parts = line.splitn(2, ' ');
    let p1 = parts.next().unwrap_or("");
    let p2 = parts.next().unwrap_or("");
    (p1, p2)
}
