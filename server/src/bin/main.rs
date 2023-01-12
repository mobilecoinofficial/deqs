// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_server::{Server, ServerConfig};
use mc_common::logger::o;
use mc_util_grpc::AdminServer;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use deqs_server::Msg;
use postage::broadcast;

#[tokio::main]
async fn main() {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    let (msg_bus_tx, msg_bus_rx) = broadcast::channel::<Msg>(1);

    let rx2 = msg_bus_rx.clone();

    // Alice and Bob will see both messages
    tokio::task::spawn(print_messages("alice", msg_bus_rx));
    tokio::task::spawn(print_messages("bob", rx2));

    let mut server = Server::new(msg_bus_tx, config.client_listen_uri.clone(), logger.clone());
    server.start().expect("Failed starting client GRPC server");

    let config_json = serde_json::to_string(&config).expect("failed to serialize config to JSON");
    let get_config_json = Arc::new(move || Ok(config_json.clone()));
    let id = config.client_listen_uri.to_string();
    let _admin_server = config.admin_listen_uri.as_ref().map(|admin_listen_uri| {
        AdminServer::start(
            None,
            admin_listen_uri,
            "DEQS".into(),
            id,
            Some(get_config_json),
            logger,
        )
        .expect("Failed starting admin server")
    });

    // Keep the server alive
    loop {
        sleep(Duration::from_secs(1)).await;
    }
}

use postage::stream::Stream;
async fn print_messages(name: &'static str, mut rx: impl Stream<Item = Msg> + Unpin) {
    while let Some(message) = rx.recv().await {
        println!("{} got a message: {:?}", name, message);
        sleep(Duration::from_secs(5)).await;
    }
}
