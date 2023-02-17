// Copyright (c) 2023 MobileCoin Inc.

use crate::{ClientService, Error, Msg};
use deqs_api::DeqsClientUri;
use deqs_quote_book_api::QuoteBook;
use futures::executor::block_on;
use mc_common::logger::{log, Logger};
use mc_util_grpc::ConnectionUriGrpcioServer;
use mc_util_uri::ConnectionUri;
use postage::broadcast::Sender;
use std::sync::Arc;

/// DEQS server
pub struct Server<OB: QuoteBook> {
    /// Message bus sender.
    msg_bus_tx: Sender<Msg>,

    /// Quote book.
    quote_book: OB,

    /// Client listen URI.
    client_listen_uri: DeqsClientUri,

    /// Logger.
    logger: Logger,

    /// Client GRPC server.
    server: Option<grpcio::Server>,
}

impl<OB: QuoteBook> Server<OB> {
    pub fn new(
        msg_bus_tx: Sender<Msg>,
        quote_book: OB,
        client_listen_uri: DeqsClientUri,
        logger: Logger,
    ) -> Self {
        Self {
            msg_bus_tx,
            quote_book,
            client_listen_uri,
            logger,
            server: None,
        }
    }

    /// Start all the grpc services and threads in the server
    pub fn start(&mut self) -> Result<(), Error> {
        let ret = self.start_helper();
        if let Err(ref err) = ret {
            log::error!(self.logger, "Server failed to start: {}", err);
            self.stop();
        }
        ret
    }

    /// Helper which gathers errors when starting server
    fn start_helper(&mut self) -> Result<(), Error> {
        self.start_client_rpc_server()?;
        Ok(())
    }

    /// Start the client RPC server
    fn start_client_rpc_server(&mut self) -> Result<(), Error> {
        log::info!(
            self.logger,
            "Starting client RPC server on {}",
            self.client_listen_uri
        );

        let health_service =
            mc_util_grpc::HealthService::new(None, self.logger.clone()).into_service();

        let client_service = ClientService::new(
            self.msg_bus_tx.clone(),
            self.quote_book.clone(),
            self.logger.clone(),
        )
        .into_service();

        let grpc_env = Arc::new(
            grpcio::EnvBuilder::new()
                .name_prefix("Deqs-Client-RPC".to_string())
                .build(),
        );

        let server_builder = grpcio::ServerBuilder::new(grpc_env)
            .register_service(health_service)
            .register_service(client_service);

        let mut server =
            server_builder.build_using_uri(&self.client_listen_uri, self.logger.clone())?;
        server.start();

        log::info!(
            self.logger,
            "Deqs Client GRPC API listening on {}",
            self.client_listen_uri.addr(),
        );

        self.server = Some(server);
        Ok(())
    }

    /// Stop the servers and threads
    /// They cannot be restarted, so this should normally be done only just
    /// before tearing down the whole server.
    pub fn stop(&mut self) {
        if let Some(mut server) = self.server.take() {
            block_on(server.shutdown()).expect("Could not stop client grpc server");
        }
    }
}

impl<OB: QuoteBook> Drop for Server<OB> {
    fn drop(&mut self) {
        self.stop();
    }
}
