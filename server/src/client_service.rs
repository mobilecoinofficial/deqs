// Copyright (c) 2023 MobileCoin Inc.

use crate::Msg;
use deqs_api::{
    deqs::{
        GetQuotesRequest, GetQuotesResponse, LiveUpdate, LiveUpdatesRequest, QuoteStatusCode,
        SubmitQuotesRequest, SubmitQuotesResponse,
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
use rayon::prelude::{IntoParallelIterator, ParallelIterator};

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

    fn submit_quotes_impl(
        &mut self,
        req: SubmitQuotesRequest,
        logger: &Logger,
    ) -> Result<SubmitQuotesResponse, RpcStatus> {
        let scis = req
            .get_quotes()
            .iter()
            .map(|sci| SignedContingentInput::try_from(sci))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| rpc_invalid_arg_error("quotes", err, logger))?;

        log::debug!(logger, "Request to submit {} orders", scis.len());

        let results = scis
            .into_par_iter()
            .map(|sci| self.order_book.add_sci(sci))
            .collect::<Vec<_>>();

        let mut status_codes = vec![];
        let mut error_messages = vec![];
        let mut orders = vec![];

        for result in results {
            match result {
                Ok(order) => {
                    status_codes.push(QuoteStatusCode::CREATED);
                    error_messages.push("".to_string());
                    orders.push((&order).into());

                    match self
                        .msg_bus_tx
                        .blocking_send(Msg::SciOrderAdded(order.clone()))
                    {
                        Ok(_) => {
                            log::info!(
                                logger,
                                "Order {} added: Max {} of token {} for {} of token {}",
                                order.id(),
                                order.base_range().end(),
                                order.pair().base_token_id,
                                order.max_counter_tokens(),
                                order.pair().counter_token_id,
                            );
                        }
                        Err(err) => {
                            log::error!(
                                logger,
                                "Failed to send SCI order added message to message bus: {:?}",
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    error_messages.push(err.to_string());
                    let order_book_error: deqs_order_book::Error = err.into();
                    status_codes.push((&order_book_error).into());
                    orders.push(Default::default());
                }
            }
        }

        Ok(SubmitQuotesResponse {
            status_codes: status_codes.into(),
            error_messages: error_messages.into(),
            orders: orders.into(),
            ..Default::default()
        })
    }

    fn get_quotes_impl(&self, _req: GetQuotesRequest) -> Result<GetQuotesResponse, RpcStatus> {
        todo!()
    }

    async fn live_updates_impl(
        mut responses: ServerStreamingSink<LiveUpdate>,
        mut msg_bus_rx: Receiver<Msg>,
    ) -> Result<(), grpcio::Error> {
        while let Some(msg) = msg_bus_rx.recv().await {
            let mut live_update = LiveUpdate::default();
            match msg {
                Msg::SciOrderAdded(order) => live_update.set_order_added((&order).into()),
                Msg::SciOrderRemoved(order) => live_update.set_order_removed((&order).into()),
            };
            responses.send((live_update, WriteFlags::default())).await?;
        }
        responses.close().await?;
        Ok(())
    }
}

impl<OB: OrderBook> DeqsClientApi for ClientService<OB> {
    fn submit_quotes(
        &mut self,
        ctx: RpcContext,
        req: SubmitQuotesRequest,
        sink: UnarySink<SubmitQuotesResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            // Build a prost response, then convert it to rpc/protobuf types and the errors
            // to rpc status codes.
            send_result(ctx, sink, self.submit_quotes_impl(req, logger), logger)
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
        _req: LiveUpdatesRequest,
        responses: ServerStreamingSink<LiveUpdate>,
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
