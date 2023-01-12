// Copyright (c) 2023 MobileCoin Inc.

use crate::Msg;
use deqs_api::{
    deqs::{
        GetQuotesRequest, GetQuotesResponse, QuoteStatusCode, SubmitQuoteRequest,
        SubmitQuoteResponse,
    },
    deqs_grpc::{create_deqs_client_api, DeqsClientApi},
};
use deqs_order_book::OrderBook;
use futures::{FutureExt, SinkExt, TryFutureExt};
use grpcio::{RpcContext, RpcStatus, ServerStreamingSink, Service, UnarySink, WriteFlags};
use mc_common::logger::{log, scoped_global_logger, Logger};
use mc_transaction_extra::SignedContingentInput;
use mc_util_grpc::{rpc_invalid_arg_error, rpc_logger, send_result};
use mc_util_metrics::SVC_COUNTERS;
use postage::{
    broadcast::{Receiver, Sender},
    prelude::Stream,
    sink::Sink,
};

/// GRPC Client service
#[derive(Clone)]
pub struct ClientService<OB: OrderBook> {
    /// Message bus sender.
    msg_bus_tx: Sender<Msg>,

    /// Order book.
    order_book: OB,

    /// Logger.
    logger: Logger,
}

impl<OB: OrderBook> ClientService<OB> {
    /// Create a new ClientService
    pub fn new(msg_bus_tx: Sender<Msg>, order_book: OB, logger: Logger) -> Self {
        Self {
            msg_bus_tx,
            order_book,
            logger,
        }
    }

    /// Convert into a grpc service
    pub fn into_service(self) -> Service {
        create_deqs_client_api(self)
    }

    fn submit_quote_impl(
        &self,
        req: SubmitQuoteRequest,
        logger: &Logger,
    ) -> Result<SubmitQuoteResponse, RpcStatus> {
        let sci: Vec<SignedContingentInput> = req
            .get_quotes()
            .iter()
            .map(|sci| SignedContingentInput::try_from(sci))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| rpc_invalid_arg_error("quotes", err, logger))?;
        //        let _order = self.order_book.add_sci()

        let logger = self.logger.clone();
        let mut msg_bus_tx = self.msg_bus_tx.clone();

        // //self.tokio_runtime.block_on(async move {
        // mc_common::logger::log::info!(logger, "tokio log eheh");
        // msg_bus_tx.blocking_send(Msg::V1).expect("send");
        // //});

        // Err(RpcStatus::new(grpcio::RpcStatusCode::DEADLINE_EXCEEDED))
        Ok(())
    }

    fn get_quotes_impl(&self, _req: GetQuotesRequest) -> Result<GetQuotesResponse, RpcStatus> {
        todo!()
    }

    async fn live_updates_impl(
        mut responses: ServerStreamingSink<SubmitQuoteResponse>,
        mut msg_bus_rx: Receiver<Msg>,
    ) -> Result<(), grpcio::Error> {
        while let Some(_msg) = msg_bus_rx.recv().await {
            responses
                .send((
                    SubmitQuoteResponse {
                        status_codes: vec![QuoteStatusCode::INVALID].into(),
                        ..Default::default()
                    },
                    WriteFlags::default(),
                ))
                .await?;
        }
        responses.close().await?;
        Ok(())
    }
}

impl<OB: OrderBook> DeqsClientApi for ClientService<OB> {
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
            send_result(ctx, sink, self.submit_quote_impl(req, logger), logger)
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

    fn live_updates(
        &mut self,
        ctx: RpcContext<'_>,
        _req: SubmitQuoteRequest,
        responses: ServerStreamingSink<SubmitQuoteResponse>,
    ) {
        let logger = self.logger.clone();
        let receiver = self.msg_bus_tx.subscribe();

        let future = Self::live_updates_impl(responses, receiver)
            .map_err(move |err: grpcio::Error| log::error!(&logger, "failed to reply: {}", err))
            // TODO: Do stuff with the error
            .map(|_| ());
        ctx.spawn(future)
    }
}
