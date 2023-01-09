// Copyright (c) 2023 MobileCoin Inc.

use clap::Parser;
use deqs_server::{Server, ServerConfig};
use mc_common::logger::o;
use std::{thread::sleep, time::Duration};

fn main() {
    let _sentry_guard = mc_common::sentry::init();
    let config = ServerConfig::parse();
    let (logger, _global_logger_guard) = mc_common::logger::create_app_logger(o!());
    mc_common::setup_panic_handler();

    let mut server = Server::new(config.client_listen_uri, logger);
    server.start().expect("Failed starting client GRPC server");

    // Keep the server alive
    loop {
        sleep(Duration::from_secs(1));
    }
}
