// Copyright (c) 2023 MobileCoin Inc.

use clap::{Parser, Subcommand};
use deqs_api::{
    deqs::{QuoteStatusCode, SubmitQuotesRequest},
    deqs_grpc::DeqsClientApiClient,
    DeqsClientUri,
};
use deqs_mc_test_utils::{create_partial_sci, create_sci};
use deqs_quote_book_api::Quote;
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_common::logger::{log, o};
use mc_ledger_db::LedgerDB;
use mc_transaction_types::TokenId;
use mc_util_grpc::ConnectionUriGrpcioChannel;
use rand::{rngs::StdRng, thread_rng, RngCore, SeedableRng};
use std::{path::PathBuf, sync::Arc};

#[derive(Subcommand)]
pub enum Command {
    /// Submit quotes to the DEQS server.
    SubmitQuotes {
        /// gRPC URI for client requests.
        #[clap(long)]
        deqs_uri: DeqsClientUri,

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
        #[clap(long, default_value = "1000")]
        counter_amount: u64,

        /// Allow partial fills
        #[clap(long)]
        allow_partial_fills: bool,

        /// Path to ledgerdb used by the server
        #[clap(long, env = "MC_LEDGER_DB")]
        ledger_db_path: PathBuf,
    },

    /// Generate a p2p keypair and write it to a file
    GenP2pKeypair {
        /// Path to write the keypair to.
        #[clap(long)]
        out: PathBuf,
    },
}

/// Command-line configuration options for the test client
#[derive(Parser)]
#[clap(version)]
pub struct Config {
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

    match config.command {
        Command::SubmitQuotes {
            deqs_uri,
            num_quotes,
            base_token_id,
            counter_token_id,
            base_amount,
            counter_amount,
            allow_partial_fills,
            ledger_db_path,
        } => {
            let ch =
                ChannelBuilder::default_channel_builder(env).connect_to_uri(&deqs_uri, &logger);
            let client_api = DeqsClientApiClient::new(ch);

            // Open the ledger db
            let ledger_db = LedgerDB::open(&ledger_db_path).expect("Could not open ledger db");

            log::info!(&logger, "Generating {} SCIs...", num_quotes);

            let scis = (0..num_quotes)
                .map(|_| {
                    if allow_partial_fills {
                        create_partial_sci(
                            base_token_id,
                            counter_token_id,
                            base_amount,
                            0,
                            0,
                            counter_amount,
                            &mut rng,
                            Some(&ledger_db),
                        )
                    } else {
                        create_sci(
                            base_token_id,
                            counter_token_id,
                            base_amount,
                            counter_amount,
                            &mut rng,
                            Some(&ledger_db),
                        )
                    }
                })
                .map(|sci| mc_api::external::SignedContingentInput::from(&sci))
                .collect::<Vec<_>>();

            log::info!(&logger, "Submitting {} SCIs...", num_quotes);

            let req = SubmitQuotesRequest {
                quotes: scis.into(),
                ..Default::default()
            };
            let resp = client_api.submit_quotes(&req).expect("submit quote failed");
            println!("{:#?}", resp);
            println!();
            for (i, (status_code, quote)) in
                resp.status_codes.iter().zip(resp.quotes.iter()).enumerate()
            {
                match status_code {
                    QuoteStatusCode::CREATED => {
                        let quote = Quote::try_from(quote).expect("invalid quote");
                        println!("{}. {}", i, hex::encode(**quote.id()));
                    }
                    err => {
                        println!("{}. {:?}", i, err);
                    }
                }
            }
        }

        Command::GenP2pKeypair { out } => {
            let keypair = deqs_p2p::libp2p::identity::Keypair::generate_ed25519();
            let bytes = keypair
                .to_protobuf_encoding()
                .expect("failed encoding to protobuf");
            std::fs::write(out, bytes).expect("failed to write keypair");
        }
    }
}
