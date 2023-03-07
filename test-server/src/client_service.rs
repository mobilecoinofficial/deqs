// Copyright (c) 2023 MobileCoin Inc.

use std::sync::{Arc, Mutex};

use deqs_api::{
    deqs::{
        GetQuotesRequest, GetQuotesResponse, LiveUpdate, LiveUpdatesRequest, SubmitQuotesRequest,
        SubmitQuotesResponse,
    },
    deqs_grpc::{create_deqs_client_api, DeqsClientApi},
};
use grpcio::{RpcContext, RpcStatus, RpcStatusCode, ServerStreamingSink, Service, UnarySink};
use mc_common::logger::Logger;
use mc_util_grpc::send_result;

/// GRPC Client service
#[derive(Clone)]
pub struct ClientService {
    /// List of SubmitQuotes requests we received
    pub submit_quotes_requests: Arc<Mutex<Vec<SubmitQuotesRequest>>>,

    /// Hardcoded response we will return for SubmitQuotes requests.
    pub submit_quotes_response: Arc<Mutex<Result<SubmitQuotesResponse, RpcStatus>>>,

    /// List of GetQuotes requests we received
    pub get_quotes_requests: Arc<Mutex<Vec<GetQuotesRequest>>>,

    /// Hardcoded response we will return for GetQuotes requests.
    pub get_quotes_response: Arc<Mutex<Result<GetQuotesResponse, RpcStatus>>>,

    /// Logger.
    logger: Logger,
}

impl ClientService {
    /// Create a new ClientService
    pub fn new(logger: Logger) -> Self {
        Self {
            submit_quotes_requests: Arc::new(Mutex::new(Vec::new())),
            submit_quotes_response: Arc::new(Mutex::new(Err(RpcStatus::new(
                RpcStatusCode::UNAVAILABLE,
            )))),
            get_quotes_requests: Arc::new(Mutex::new(Vec::new())),
            get_quotes_response: Arc::new(Mutex::new(Err(RpcStatus::new(
                RpcStatusCode::UNAVAILABLE,
            )))),
            logger,
        }
    }

    /// Convert into a grpc service
    pub fn into_service(self) -> Service {
        create_deqs_client_api(self)
    }
}

impl DeqsClientApi for ClientService {
    fn submit_quotes(
        &mut self,
        ctx: RpcContext,
        req: SubmitQuotesRequest,
        sink: UnarySink<SubmitQuotesResponse>,
    ) {
        self.submit_quotes_requests.lock().unwrap().push(req);

        send_result(
            ctx,
            sink,
            self.submit_quotes_response.lock().unwrap().clone(),
            &self.logger,
        )
    }

    fn get_quotes(
        &mut self,
        ctx: RpcContext,
        req: GetQuotesRequest,
        sink: UnarySink<GetQuotesResponse>,
    ) {
        self.get_quotes_requests.lock().unwrap().push(req);

        send_result(
            ctx,
            sink,
            self.get_quotes_response.lock().unwrap().clone(),
            &self.logger,
        )
    }

    fn live_updates(
        &mut self,
        _ctx: RpcContext<'_>,
        _req: LiveUpdatesRequest,
        _responses: ServerStreamingSink<LiveUpdate>,
    ) {
        unimplemented!()
    }
}
