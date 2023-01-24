// Copyright (c) 2023 MobileCoin Inc.

//! Configuration parameters for the DEQS server

use clap::Parser;
use deqs_api::DeqsClientUri;
use mc_util_uri::AdminUri;
use serde::Serialize;

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

    /// TODO
    #[clap(long = "peer", env = "MC_PEER")]
    pub peers: Vec<String>,

    /// TODO
    #[clap(long = "peer-id", env = "MC_PEER_ID")]
    pub peer_ids: Vec<String>,
}
