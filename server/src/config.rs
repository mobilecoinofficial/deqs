// Copyright (c) 2023 MobileCoin Inc.

//! Configuration parameters for the DEQS server

use clap::Parser;
use deqs_api::DeqsClientUri;

/// Command-line configuration options for the DEQS server
#[derive(Parser)]
#[clap(version)]
pub struct ServerConfig {
    /// gRPC listening URI for client requests.
    #[clap(long, env = "MC_CLIENT_LISTEN_URI")]
    pub client_listen_uri: DeqsClientUri,
}
