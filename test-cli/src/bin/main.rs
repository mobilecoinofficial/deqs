// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_api::{deqs::SubmitQuotesRequest, deqs_grpc::DeqsClientApiClient, DeqsClientUri};
use deqs_mc_test_utils::create_sci;
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_common::logger::o;
use mc_transaction_types::TokenId;
use mc_util_grpc::ConnectionUriGrpcioChannel;
use rand::thread_rng;
use std::sync::Arc;

/// Command-line configuration options for the test client
#[derive(Parser)]
#[clap(version)]
pub struct Config {
    /// gRPC URI for client requests.
    #[clap(long, env = "MC_CLIENT_LISTEN_URI")]
    pub deqs_uri: DeqsClientUri,
}

fn main() {
    let config = Config::parse();
    let env = Arc::new(EnvBuilder::new().name_prefix("deqs-client-grpc").build());
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    let mut rng = thread_rng();

    let ch = ChannelBuilder::default_channel_builder(env).connect_to_uri(&config.deqs_uri, &logger);
    let client_api = DeqsClientApiClient::new(ch);

    let sci = create_sci(
        TokenId::from(43432),
        TokenId::from(6886868),
        10,
        30,
        &mut rng,
    );

    let req = SubmitQuotesRequest {
        quotes: vec![(&sci).into()].into(),
        ..Default::default()
    };
    let resp = client_api.submit_quotes(&req).expect("submit quote failed");
    println!("{:#?}", resp);
}
