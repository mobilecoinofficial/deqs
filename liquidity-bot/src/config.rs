// Copyright (c) 2023 MobileCoin Inc.

//! Configuration parameters for a liquidity bot

use std::path::PathBuf;

use clap::Parser;
use deqs_api::DeqsClientUri;
use mc_transaction_types::TokenId;
use mc_util_uri::AdminUri;
use rust_decimal::Decimal;
use serde::Serialize;

/// Command-line configuration options for the DEQS server
#[derive(Parser, Serialize)]
#[clap(version)]
pub struct Config {
    /// DEQS server to connect to
    #[clap(long, env = "DEQS_URI")]
    pub deqs: DeqsClientUri,

    /// Optional admin listening URI.
    #[clap(long, env = "MC_ADMIN_LISTEN_URI")]
    pub admin_listen_uri: Option<AdminUri>,

    /// Token id to offer
    #[clap(long, env = "DEQS_BASE_TOKEN_ID")]
    pub base_token_id: TokenId,

    /// Token id that needs to be paid back to the bot to get the base token.
    #[clap(long, env = "DEQS_COUNTER_TOKEN_ID")]
    pub counter_token_id: TokenId,

    /// The base to counter swap rate we are offering.
    /// This specifies how many counter tokens are needed to get one base token.
    #[clap(long, env = "DEQS_SWAP_RATE")]
    pub swap_rate: Decimal,

    /// Ledger DB path
    #[clap(long, env = "MC_LEDGER_DB")]
    pub ledger_db: PathBuf,

    /// Account key file
    #[clap(long, env = "MC_ACCOUNT_KEY")]
    pub account_key: PathBuf,

    /// Wallet state file
    #[clap(long, env = "WALLET_DB")]
    pub wallet_db: PathBuf,
}
