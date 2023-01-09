// Copyright (c) 2023 MobileCoin Inc.

use deqs_api::{
    deqs::{GetQuotesRequest, GetQuotesResponse, SubmitQuoteRequest, SubmitQuoteResponse},
    deqs_grpc::{create_deqs_client_api, DeqsClientApi},
};
use grpcio::{RpcContext, RpcStatus, Service, UnarySink};
use mc_common::logger::{scoped_global_logger, Logger};
use mc_util_grpc::{rpc_logger, send_result};
use mc_util_metrics::SVC_COUNTERS;

/// GRPC Client service
#[derive(Clone)]
pub struct ClientService {
    /// Logger.
    #[allow(dead_code)]
    logger: Logger,
}

impl ClientService {
    /// Create a new ClientService
    pub fn new(logger: Logger) -> Self {
        Self { logger }
    }

    /// Convert into a grpc service
    pub fn into_service(self) -> Service {
        create_deqs_client_api(self)
    }

    fn submit_quote_impl(
        &self,
        _req: SubmitQuoteRequest,
    ) -> Result<SubmitQuoteResponse, RpcStatus> {
        todo!()
    }

    fn get_quotes_impl(&self, _req: GetQuotesRequest) -> Result<GetQuotesResponse, RpcStatus> {
        todo!()
    }
}

impl DeqsClientApi for ClientService {
    fn submit_quote(
        &mut self,
        ctx: RpcContext,
        req: SubmitQuoteRequest,
        sink: UnarySink<SubmitQuoteResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            // Build a prost response, then convert it to rpc/protobuf types and the errors
            // to rpc status codes.
            send_result(ctx, sink, self.submit_quote_impl(req), logger)
        })
    }

    fn get_quotes(
        &mut self,
        ctx: RpcContext,
        req: GetQuotesRequest,
        sink: UnarySink<GetQuotesResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            // Build a prost response, then convert it to rpc/protobuf types and the errors
            // to rpc status codes.
            send_result(ctx, sink, self.get_quotes_impl(req), logger)
        })
    }
}
