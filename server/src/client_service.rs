// Copyright (c) 2023 MobileCoin Inc.

use crate::Msg;
use deqs_api::{
    deqs::{
        GetQuotesRequest, GetQuotesResponse, LiveUpdate, LiveUpdatesRequest, QuoteStatusCode,
        RemoveOrderRequest, RemoveOrderResponse, SubmitQuotesRequest, SubmitQuotesResponse,
    },
    deqs_grpc::{create_deqs_client_api, DeqsClientApi},
};
use deqs_order_book::{Error as OrderBookError, OrderBook, OrderId, Pair};
use futures::{FutureExt, SinkExt};
use grpcio::{
    RpcContext, RpcStatus, RpcStatusCode, ServerStreamingSink, Service, UnarySink, WriteFlags,
};
use mc_common::logger::{log, scoped_global_logger, Logger};
use mc_transaction_extra::SignedContingentInput;
use mc_util_grpc::{rpc_internal_error, rpc_invalid_arg_error, rpc_logger, send_result};
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
            .map(SignedContingentInput::try_from)
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
            status_codes,
            error_messages: error_messages.into(),
            orders: orders.into(),
            ..Default::default()
        })
    }

    fn get_quotes_impl(
        &self,
        mut req: GetQuotesRequest,
        logger: &Logger,
    ) -> Result<GetQuotesResponse, RpcStatus> {
        // If no range is specified, adjust so that it covers the entire range.
        if req.base_range_min == 0 && req.base_range_max == 0 {
            req.base_range_max = u64::MAX;
        }

        let orders = self
            .order_book
            .get_orders(
                &Pair::from(req.get_pair()),
                req.base_range_min..=req.base_range_max,
                req.limit as usize,
            )
            .map_err(|err| rpc_internal_error("get_quotes", err, logger))?;
        log::debug!(
            logger,
            "Request to get {:?} orders returning {} orders",
            req,
            orders.len()
        );
        Ok(GetQuotesResponse {
            orders: orders.into_iter().map(|order| (&order).into()).collect(),
            ..Default::default()
        })
    }

    fn remove_order_impl(
        &mut self,
        req: RemoveOrderRequest,
        logger: &Logger,
    ) -> Result<RemoveOrderResponse, RpcStatus> {
        let order_id = OrderId::try_from(req.get_order_id())
            .map_err(|err| rpc_invalid_arg_error("order_id", err, &self.logger))?;

        match self
            .order_book
            .remove_order_by_id(&order_id)
            .map_err(|err| err.into())
        {
            Ok(order) => {
                log::info!(self.logger, "Order {} removed", order.id());
                if let Err(err) = self
                    .msg_bus_tx
                    .blocking_send(Msg::SciOrderRemoved(order_id))
                {
                    log::error!(
                        logger,
                        "Failed to send SCI order {} removed message to message bus: {:?}",
                        order.id(),
                        err
                    );
                }

                let mut resp = RemoveOrderResponse::default();
                resp.set_order((&order).into());
                Ok(resp)
            }
            Err(OrderBookError::OrderNotFound) => Err(RpcStatus::new(RpcStatusCode::NOT_FOUND)),
            Err(err) => {
                log::error!(logger, "Failed to remove order {}: {:?}", order_id, err);
                Err(rpc_internal_error("remove_order", err, &self.logger))
            }
        }
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
            send_result(ctx, sink, self.get_quotes_impl(req, logger), logger)
        })
    }

    fn remove_order(
        &mut self,
        ctx: RpcContext,
        req: RemoveOrderRequest,
        sink: UnarySink<RemoveOrderResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.remove_order_impl(req, logger), logger)
        })
    }

    fn live_updates(
        &mut self,
        ctx: RpcContext<'_>,
        _req: LiveUpdatesRequest,
        responses: ServerStreamingSink<LiveUpdate>,
    ) {
        let receiver = self.msg_bus_tx.subscribe();

        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            let logger = logger.clone();

            let future = Self::live_updates_impl(responses, receiver).map(move |future_result| {
                if let Err(err) = future_result {
                    log::error!(logger, "live_updates_impl failed: {:?}", err);
                }
            });
            ctx.spawn(future)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deqs_api::{deqs_grpc::DeqsClientApiClient, DeqsClientUri};
    use deqs_mc_test_utils::create_sci;
    use deqs_order_book::{InMemoryOrderBook, Order};
    use grpcio::{ChannelBuilder, EnvBuilder, Server, ServerBuilder};
    use mc_common::logger::test_with_logger;
    use mc_transaction_types::TokenId;
    use mc_util_grpc::{ConnectionUriGrpcioChannel, ConnectionUriGrpcioServer};
    use postage::broadcast;
    use rand::{rngs::StdRng, SeedableRng};
    use std::{str::FromStr, sync::Arc};

    fn create_test_client_and_server<OB: OrderBook>(
        order_book: &OB,
        logger: &Logger,
    ) -> (DeqsClientApiClient, Server, Receiver<Msg>) {
        let server_env = Arc::new(EnvBuilder::new().build());
        let (msg_bus_tx, msg_bus_rx) = broadcast::channel::<Msg>(1000);

        let client_service =
            ClientService::new(msg_bus_tx, order_book.clone(), logger.clone()).into_service();
        let server_builder = ServerBuilder::new(server_env)
            .register_service(client_service)
            .bind_using_uri(
                &DeqsClientUri::from_str("insecure-deqs://127.0.0.1:0").unwrap(),
                logger.clone(),
            );
        let mut server = server_builder.build().expect("build server");
        server.start();

        let client_env = Arc::new(EnvBuilder::new().build());
        let ch = ChannelBuilder::default_channel_builder(client_env).connect_to_uri(
            &DeqsClientUri::from_str(&format!(
                "insecure-deqs://127.0.0.1:{}",
                server.bind_addrs().next().unwrap().1
            ))
            .unwrap(),
            &logger,
        );
        let client_api = DeqsClientApiClient::new(ch);

        (client_api, server, msg_bus_rx)
    }

    #[test_with_logger]
    fn submit_quotes_add_orders(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let order_book = InMemoryOrderBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&order_book, &logger);

        let scis = (0..10)
            .map(|_| create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng))
            .map(|sci| mc_api::external::SignedContingentInput::from(&sci))
            .collect::<Vec<_>>();

        let req = SubmitQuotesRequest {
            quotes: scis.clone().into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");

        assert_eq!(
            resp.get_status_codes(),
            vec![QuoteStatusCode::CREATED; scis.len()]
        );
        assert_eq!(resp.get_error_messages(), vec![""; scis.len()]);
        let mut orders = resp
            .get_orders()
            .iter()
            .map(|o| Order::try_from(o).unwrap())
            .collect::<Vec<_>>();

        let mut orders2 = order_book
            .get_orders(&pair, .., 0)
            .unwrap()
            .into_iter()
            .rev() // Orders returned in newest to oldest order
            .collect::<Vec<_>>();

        // Since orders are added in parallel, the exact order at which they get added
        // is not determinstic.
        orders.sort();
        orders2.sort();

        assert_eq!(orders2, orders);
    }

    #[test_with_logger]
    fn submit_quotes_refuses_duplicate_order(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let order_book = InMemoryOrderBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&order_book, &logger);

        let sci = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);

        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(
            resp.status_codes,
            vec![QuoteStatusCode::ORDER_ALREADY_EXISTS]
        );
    }

    #[test_with_logger]
    fn get_quotes_filter_correctly(logger: Logger) {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let order_book = InMemoryOrderBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&order_book, &logger);

        let sci1 = create_sci(TokenId::from(1), TokenId::from(2), 10, 20, &mut rng);
        let sci2 = create_sci(TokenId::from(1), TokenId::from(2), 10, 50, &mut rng);
        let sci3 = create_sci(TokenId::from(3), TokenId::from(4), 10, 20, &mut rng);
        let sci4 = create_sci(TokenId::from(3), TokenId::from(4), 12, 50, &mut rng);

        let scis = [&sci1, &sci2, &sci3, &sci4]
            .into_iter()
            .map(|sci| mc_api::external::SignedContingentInput::from(sci))
            .collect::<Vec<_>>();

        let req = SubmitQuotesRequest {
            quotes: scis.clone().into(),
            ..Default::default()
        };
        client_api.submit_quotes(&req).expect("submit quote failed");

        // Correct pair is return without any extra filtering
        let pair = deqs_api::deqs::Pair {
            base_token_id: 3,
            counter_token_id: 4,
            ..Default::default()
        };

        let mut req = GetQuotesRequest::default();
        req.set_pair(pair);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_orders()
            .iter()
            .map(|order| SignedContingentInput::try_from(order.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3.clone(), sci4.clone()]);

        // Limit is respected
        req.set_limit(1);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_orders()
            .iter()
            .map(|order| SignedContingentInput::try_from(order.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3.clone()]);

        // Base range is respected
        req.set_limit(u64::MAX);
        req.set_base_range_max(10);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_orders()
            .iter()
            .map(|order| SignedContingentInput::try_from(order.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3.clone()]);

        req.set_base_range_min(11);
        req.set_base_range_max(12);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_orders()
            .iter()
            .map(|order| SignedContingentInput::try_from(order.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci4.clone()]);

        req.set_base_range_min(10);
        req.set_base_range_max(12);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_orders()
            .iter()
            .map(|order| SignedContingentInput::try_from(order.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3, sci4]);
    }

    #[test_with_logger]
    fn remove_order_works(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let order_book = InMemoryOrderBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&order_book, &logger);

        let sci = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);

        // Order is present
        let orders = order_book.get_orders(&pair, .., 0).unwrap();
        assert_eq!(orders.len(), 1);

        // Remove it.
        let order_id = deqs_api::deqs::OrderId::from(orders[0].id());
        let mut req = RemoveOrderRequest::default();
        req.set_order_id(order_id);

        let resp = client_api.remove_order(&req).expect("remove order failed");
        assert_eq!(resp.get_order(), &(&orders[0]).into());

        // Try again, should fail.
        assert!(
            matches!(client_api.remove_order(&req), Err(grpcio::Error::RpcFailure(status)) if status.code() == grpcio::RpcStatusCode::NOT_FOUND)
        );

        // Invalid order id should fail.
        let req = RemoveOrderRequest::default();
        assert!(
            matches!(client_api.remove_order(&req), Err(grpcio::Error::RpcFailure(status)) if status.code() == grpcio::RpcStatusCode::INVALID_ARGUMENT)
        );
    }

    // TODO add test for streaming
}
