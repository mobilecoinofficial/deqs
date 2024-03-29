// Copyright (c) 2023 MobileCoin Inc.

use crate::{client_service::SciValidator, ClientService, Error, Msg};
use deqs_api::DeqsClientUri;
use deqs_quote_book_api::{Error as QuoteBookError, QuoteBook};
use futures::executor::block_on;
use mc_common::{
    logger::{log, Logger},
    HashMap,
};
use mc_transaction_types::TokenId;
use mc_util_grpc::ConnectionUriGrpcioServer;
use mc_util_uri::ConnectionUri;
use postage::broadcast::Sender;
use std::sync::Arc;

/// DEQS server
pub struct GrpcServer<OB: QuoteBook> {
    /// Message bus sender.
    msg_bus_tx: Sender<Msg>,

    /// Quote book.
    quote_book: OB,

    /// Client listen URI. This is the URI that we are asked to listen on.
    client_listen_uri: DeqsClientUri,

    /// Logger.
    logger: Logger,

    /// Client GRPC server.
    server: Option<grpcio::Server>,

    /// Map of token id to minimum quote size
    quote_minimum_map: HashMap<TokenId, u64>,

    /// The address we are listening on, once the server is started.
    /// This might differ from the client_listen_uri if the client_listen_uri
    /// port is 0 and we are choosing an available port when we start.
    actual_listen_uri: Option<DeqsClientUri>,
}

impl<OB: QuoteBook> GrpcServer<OB> {
    pub fn new(
        msg_bus_tx: Sender<Msg>,
        quote_book: OB,
        client_listen_uri: DeqsClientUri,
        quote_minimum_map: HashMap<TokenId, u64>,
        logger: Logger,
    ) -> Self {
        Self {
            msg_bus_tx,
            quote_book,
            client_listen_uri,
            logger,
            server: None,
            quote_minimum_map,
            actual_listen_uri: None,
        }
    }

    /// Start all the grpc services and threads in the server
    #[allow(clippy::result_large_err)]
    pub fn start(&mut self) -> Result<(), Error> {
        let ret = self.start_helper();
        if let Err(ref err) = ret {
            log::error!(self.logger, "GrpcServer failed to start: {}", err);
            self.stop();
        }
        ret
    }

    /// Helper which gathers errors when starting server
    #[allow(clippy::result_large_err)]
    fn start_helper(&mut self) -> Result<(), Error> {
        self.start_client_rpc_server()?;
        Ok(())
    }

    /// Start the client RPC server
    #[allow(clippy::result_large_err)]
    fn start_client_rpc_server(&mut self) -> Result<(), Error> {
        log::info!(
            self.logger,
            "Starting client RPC server on {}",
            self.client_listen_uri
        );

        let health_service =
            mc_util_grpc::HealthService::new(None, self.logger.clone()).into_service();
        let quote_minimum_map = self.quote_minimum_map.clone();

        let sci_validator: SciValidator = Arc::new(move |sci| {
            let base_token_id = TokenId::from(sci.pseudo_output_amount.token_id);
            let base_amount = sci.pseudo_output_amount.value;
            let minimum_for_token_id = *quote_minimum_map.get(&base_token_id).unwrap_or(&0);
            if base_amount < minimum_for_token_id {
                return Err(QuoteBookError::UnsupportedSci(
                    format!("Quote volume is too small for deqs. Quotes with base_token: {base_token_id} require a minimum of: {minimum_for_token_id}"
                )));
            }
            Ok(())
        });

        let client_service = ClientService::new(
            self.msg_bus_tx.clone(),
            self.quote_book.clone(),
            sci_validator,
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

        let server_creds = grpcio::ServerBuilder::server_credentials_from_uri(
            &self.client_listen_uri,
            &self.logger,
        );

        let mut server = server_builder.build()?;
        let port = server.add_listening_port(self.client_listen_uri.addr(), server_creds)?;
        server.start();

        let mut actual_listen_uri = self.client_listen_uri.clone();
        actual_listen_uri.set_port(port);

        log::info!(
            self.logger,
            "Deqs Client GRPC API listening on {}",
            actual_listen_uri
        );

        self.server = Some(server);
        self.actual_listen_uri = Some(actual_listen_uri);

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

    /// Get our actual listening uri, if available.
    pub fn actual_listen_uri(&self) -> Option<DeqsClientUri> {
        self.actual_listen_uri.clone()
    }
}

impl<OB: QuoteBook> Drop for GrpcServer<OB> {
    fn drop(&mut self) {
        self.stop();
    }
}
