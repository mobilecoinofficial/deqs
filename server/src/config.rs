// Copyright (c) 2023 MobileCoin Inc.

//! Configuration parameters for the DEQS server

use clap::Parser;
use deqs_api::DeqsClientUri;
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
}
