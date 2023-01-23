// Copyright (c) 2023 MobileCoin Inc.

use clap::{Parser, Subcommand};
use deqs_api::{
    deqs::{QuoteStatusCode, RemoveQuoteRequest, SubmitQuotesRequest},
    deqs_grpc::DeqsClientApiClient,
    DeqsClientUri,
};
use deqs_mc_test_utils::{create_partial_sci, create_sci};
use deqs_quote_book::{Quote, QuoteId};
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_common::logger::{log, o};
use mc_transaction_types::TokenId;
use mc_util_grpc::ConnectionUriGrpcioChannel;
use rand::{rngs::StdRng, thread_rng, RngCore, SeedableRng};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
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
        #[clap(long, default_value = "1000")]
        counter_amount: u64,

        /// Allow partial fills
        #[clap(long)]
        allow_partial_fills: bool,
    },

    /// Remove an quote.
    RemoveQuote {
        /// Quote id.
        #[clap(
            long,
            value_parser = mc_util_parse::parse_hex::<[u8; 32]>
        )]
        quote_id: [u8; 32],
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
            log::info!(&logger, "Generating {} SCIs...", num_quotes);

            // We can't share our rng with the Rayon threads, but we still want a
            // determinstic way to generate SCIs. This little hack allows us to
            // do that by generating a unique seed for each SCI we will be generating.
            let rng_seeds = (0..num_quotes)
                .map(|_| {
                    let mut seed_bytes = [0; 32];
                    rng.fill_bytes(&mut seed_bytes);
                    seed_bytes
                })
                .collect::<Vec<_>>();

            let scis = rng_seeds
                .into_par_iter()
                .map(|rng_seed| {
                    let mut rng: StdRng = SeedableRng::from_seed(rng_seed);

                    if allow_partial_fills {
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

        Command::RemoveQuote { quote_id } => {
            let mut req = RemoveQuoteRequest::default();
            req.set_quote_id((&QuoteId(quote_id)).into());
            let resp = client_api.remove_quote(&req).expect("remove quote failed");
            println!("{:#?}", resp);
        }
    }
}
