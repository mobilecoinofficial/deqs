// Copyright (c) 2023 MobileCoin Inc.

//! Configuration parameters for the DEQS server

use clap::Parser;
use deqs_api::DeqsClientUri;
use deqs_p2p::libp2p::Multiaddr;
use mc_util_uri::AdminUri;
use serde::Serialize;
use std::path::PathBuf;

/// Command-line configuration options for the DEQS server
#[derive(Parser, Serialize)]
#[clap(version)]
pub struct ServerConfig {
    /// gRPC listening URI for client requests.
    #[clap(long, env = "MC_CLIENT_LISTEN_URI")]
    pub client_listen_uri: DeqsClientUri,

    /// Optional admin listening URI.
    #[clap(long, env = "MC_ADMIN_LISTEN_URI")]
    pub admin_listen_uri: Option<AdminUri>,

    /// Path to ledgerdb
    #[clap(long = "ledger-db", env = "MC_LEDGER_DB")]
    pub ledger_db: PathBuf,

    /// Bootstrap p2p peers. Need to include a `/p2p/<hash>` postfix, e.g.
    /// `/ip4/127.0.0.1/tcp/49946/p2p/
    /// 12D3KooWDExx59EUZCN3kBJXKNHHmfWb1HShvMmzGxGWWpeWXHEp`
    #[clap(long = "p2p-bootstrap-peer", env = "MC_P2P_BOOTSTRAP_PEER")]
    pub p2p_bootstrap_peers: Vec<Multiaddr>,

    /// The p2p listen address. Provide in order to enable p2p.
    #[clap(long = "p2p-listen", env = "MC_P2P_LISTEN")]
    pub p2p_listen: Option<Multiaddr>,

    /// External p2p address to announce.
    #[clap(long = "p2p-external-address", env = "MC_P2P_EXTERNAL_ADDRESS")]
    pub p2p_external_address: Option<Multiaddr>,

    /// The p2p keypair file. A random one will be generated if not provided.
    #[clap(long = "p2p-keypair", env = "MC_P2P_KEYPAIR")]
    pub p2p_keypair_path: Option<PathBuf>,
}
