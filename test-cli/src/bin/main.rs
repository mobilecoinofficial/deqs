// Copyright (c) 2023 MobileCoin Inc.

use clap::{Parser, Subcommand};
use deqs_api::{deqs::SubmitQuotesRequest, deqs_grpc::DeqsClientApiClient, DeqsClientUri};
use deqs_mc_test_utils::{create_partial_sci, create_sci};
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_common::logger::o;
use mc_transaction_types::TokenId;
use mc_util_grpc::ConnectionUriGrpcioChannel;
use rand::{rngs::StdRng, thread_rng, RngCore, SeedableRng};
use std::sync::Arc;

#[derive(Subcommand)]
pub enum Command {
    /// Submit quotes to the DEQS server.
    SubmitQuotes {
        /// Number of quotes to submit.
        #[clap(long, default_value = "1")]
        num_quotes: u64,

        /// Base token id
        #[clap(long, default_value = "0")]
        base_token_id: TokenId,

        /// Counter token id
        #[clap(long, default_value = "1")]
        counter_token_id: TokenId,

        /// Base token amount
        #[clap(long, default_value = "100")]
        base_amount: u64,

        /// Counter token amount
        #[clap(long, default_value = "100")]
        counter_amount: u64,

        /// Allow partial fills
        #[clap(long)]
        allow_partial_fills: bool,
    },
}

/// Command-line configuration options for the test client
#[derive(Parser)]
#[clap(version)]
pub struct Config {
    /// gRPC URI for client requests.
    #[clap(long, env = "MC_CLIENT_LISTEN_URI")]
    pub deqs_uri: DeqsClientUri,

    /// Optional seed for the rng.
    #[clap(long)]
    pub rng_seed: Option<u64>,

    #[clap(subcommand)]
    pub command: Command,
}

fn main() {
    let config = Config::parse();
    let env = Arc::new(EnvBuilder::new().name_prefix("deqs-client-grpc").build());
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());

    let mut seed_bytes = [0; 32];
    if let Some(seed) = config.rng_seed {
        seed_bytes[..8].copy_from_slice(&seed.to_be_bytes());
    } else {
        thread_rng().fill_bytes(&mut seed_bytes);
    };
    let mut rng: StdRng = SeedableRng::from_seed(seed_bytes);

    let ch = ChannelBuilder::default_channel_builder(env).connect_to_uri(&config.deqs_uri, &logger);
    let client_api = DeqsClientApiClient::new(ch);

    match config.command {
        Command::SubmitQuotes {
            num_quotes,
            base_token_id,
            counter_token_id,
            base_amount,
            counter_amount,
            allow_partial_fills,
        } => {
            for _ in 0..num_quotes {
                let sci = if allow_partial_fills {
                    create_partial_sci(
                        base_token_id,
                        counter_token_id,
                        base_amount,
                        0,
                        0,
                        counter_amount,
                        &mut rng,
                    )
                } else {
                    create_sci(
                        base_token_id,
                        counter_token_id,
                        base_amount,
                        counter_amount,
                        &mut rng,
                    )
                };

                let req = SubmitQuotesRequest {
                    quotes: vec![(&sci).into()].into(),
                    ..Default::default()
                };
                let resp = client_api.submit_quotes(&req).expect("submit quote failed");
                println!("{:#?}", resp);
            }
        }
    }
}
