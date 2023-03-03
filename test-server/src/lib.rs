// Copyright (c) 2023 MobileCoin Inc.

//! A GRPC server implementing the DEQS GRPC API that can be used for unit
//! testing DEQS clients/bots.

mod client_service;

use std::{str::FromStr, sync::Arc};

use client_service::ClientService;
use deqs_api::{
    deqs::{GetQuotesRequest, GetQuotesResponse, SubmitQuotesRequest, SubmitQuotesResponse},
    DeqsClientUri,
};
use futures::executor::block_on;
use grpcio::{RpcStatus, ServerCredentials};
use mc_common::logger::Logger;

pub struct DeqsTestServer {
    server: Option<grpcio::Server>,
    client_service: ClientService,
    client_uri: DeqsClientUri,
}

impl DeqsTestServer {
    pub fn start(logger: Logger) -> Self {
        let client_service = ClientService::new(logger);

        let grpc_env = Arc::new(
            grpcio::EnvBuilder::new()
                .name_prefix("Deqs-Client-RPC".to_string())
                .build(),
        );

        let server_builder = grpcio::ServerBuilder::new(grpc_env)
            .register_service(client_service.clone().into_service());

        let mut server = server_builder.build().unwrap();
        let port = server
            .add_listening_port("127.0.0.1:0", ServerCredentials::insecure())
            .unwrap();
        server.start();

        Self {
            server: Some(server),
            client_service,
            client_uri: DeqsClientUri::from_str(&format!("insecure-deqs://127.0.0.1:{}", port))
                .unwrap(),
        }
    }

    pub fn set_submit_quotes_response(&self, resp: Result<SubmitQuotesResponse, RpcStatus>) {
        *self.client_service.submit_quotes_response.lock().unwrap() = resp;
    }

    pub fn set_get_quotes_response(&self, resp: Result<GetQuotesResponse, RpcStatus>) {
        *self.client_service.get_quotes_response.lock().unwrap() = resp;
    }

    pub fn pop_submit_quotes_requests(&self) -> Vec<SubmitQuotesRequest> {
        self.client_service
            .submit_quotes_requests
            .lock()
            .unwrap()
            .drain(..)
            .collect()
    }

    pub fn pop_get_quotes_requests(&self) -> Vec<GetQuotesRequest> {
        self.client_service
            .get_quotes_requests
            .lock()
            .unwrap()
            .drain(..)
            .collect()
    }

    pub fn client_uri(&self) -> DeqsClientUri {
        self.client_uri.clone()
    }
}

impl Drop for DeqsTestServer {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            block_on(server.shutdown()).expect("Could not stop test grpc server");
        }
    }
}
