// Copyright (c) 2023 MobileCoin Inc.

use crate::{Msg, NotifyingQuoteBook, SVC_COUNTERS};
use deqs_api::{
    deqs::{
        GetQuotesRequest, GetQuotesResponse, LiveUpdate, LiveUpdatesRequest, QuoteStatusCode,
        RemoveQuoteRequest, RemoveQuoteResponse, SubmitQuotesRequest, SubmitQuotesResponse,
    },
    deqs_grpc::{create_deqs_client_api, DeqsClientApi},
};
use deqs_quote_book_api::{Error as QuoteBookError, Pair, QuoteBook, QuoteId};
use futures::{FutureExt, SinkExt};
use grpcio::{
    RpcContext, RpcStatus, RpcStatusCode, ServerStreamingSink, Service, UnarySink, WriteFlags,
};
use mc_common::logger::{log, scoped_global_logger, Logger};
use mc_transaction_extra::SignedContingentInput;
use mc_util_grpc::{rpc_internal_error, rpc_invalid_arg_error, rpc_logger, send_result};
use postage::{
    broadcast::{Receiver, Sender},
    prelude::Stream,
    sink::Sink,
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::time::{SystemTime, UNIX_EPOCH};

/// GRPC Client service
#[derive(Clone)]
pub struct ClientService<QB: QuoteBook> {
    /// Message bus sender.
    msg_bus_tx: Sender<Msg>,

    /// Quote book.
    quote_book: NotifyingQuoteBook<QB>,

    /// Logger.
    logger: Logger,
}

impl<QB: QuoteBook> ClientService<QB> {
    /// Create a new ClientService
    pub fn new(
        msg_bus_tx: Sender<Msg>,
        quote_book: NotifyingQuoteBook<QB>,
        logger: Logger,
    ) -> Self {
        Self {
            msg_bus_tx,
            quote_book,
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
        // Capture timestamp before we do anything, this both ensures quotes are created
        // with the time we actually began processing the request, and that all quotes
        // are created with the same time so ordering is determinstic.
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| rpc_internal_error("submit_quotes", err, logger))?
            .as_nanos()
            .try_into()
            .map_err(|err| rpc_internal_error("submit_quotes", err, logger))?;

        let scis = req
            .get_quotes()
            .iter()
            .map(SignedContingentInput::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| rpc_invalid_arg_error("quotes", err, logger))?;

        log::debug!(logger, "Request to submit {} quotes", scis.len());

        let results = scis
            .into_par_iter()
            .map(|sci| self.quote_book.add_sci(sci, Some(timestamp)))
            .collect::<Vec<_>>();

        let mut status_codes = vec![];
        let mut error_messages = vec![];
        let mut quotes = vec![];

        for result in results {
            match result {
                Ok(quote) => {
                    status_codes.push(QuoteStatusCode::CREATED);
                    error_messages.push("".to_string());
                    quotes.push((&quote).into());

                    match self
                        .msg_bus_tx
                        .blocking_send(Msg::SciQuoteAdded(quote.clone()))
                    {
                        Ok(_) => {
                            log::info!(
                                logger,
                                "Quote {} added: Max {} of token {} for {} of token {}",
                                quote.id(),
                                quote.base_range().end(),
                                quote.pair().base_token_id,
                                quote.max_counter_tokens(),
                                quote.pair().counter_token_id,
                            );
                        }
                        Err(err) => {
                            log::error!(
                                logger,
                                "Failed to send SCI quote added message to message bus: {:?}",
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    error_messages.push(err.to_string());
                    status_codes.push((&err).into());
                    quotes.push(Default::default());
                }
            }
        }

        Ok(SubmitQuotesResponse {
            status_codes,
            error_messages: error_messages.into(),
            quotes: quotes.into(),
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

        let quotes = self
            .quote_book
            .get_quotes(
                &Pair::from(req.get_pair()),
                req.base_range_min..=req.base_range_max,
                req.limit as usize,
            )
            .map_err(|err| rpc_internal_error("get_quotes", err, logger))?;
        log::debug!(
            logger,
            "Request to get {:?} quotes returning {} quotes",
            req,
            quotes.len()
        );
        Ok(GetQuotesResponse {
            quotes: quotes.into_iter().map(|quote| (&quote).into()).collect(),
            ..Default::default()
        })
    }

    fn remove_quote_impl(
        &mut self,
        req: RemoveQuoteRequest,
        logger: &Logger,
    ) -> Result<RemoveQuoteResponse, RpcStatus> {
        let quote_id = QuoteId::try_from(req.get_quote_id())
            .map_err(|err| rpc_invalid_arg_error("quote_id", err, &self.logger))?;

        match self.quote_book.remove_quote_by_id(&quote_id) {
            Ok(quote) => {
                log::info!(self.logger, "Quote {} removed", quote.id());

                let mut resp = RemoveQuoteResponse::default();
                resp.set_quote((&quote).into());

                if let Err(err) = self.msg_bus_tx.blocking_send(Msg::SciQuoteRemoved(quote)) {
                    log::error!(
                        logger,
                        "Failed to send SCI quote {} removed message to message bus: {:?}",
                        quote_id,
                        err
                    );
                }

                Ok(resp)
            }
            Err(QuoteBookError::QuoteNotFound) => Err(RpcStatus::new(RpcStatusCode::NOT_FOUND)),
            Err(err) => {
                log::error!(logger, "Failed to remove quote {}: {:?}", quote_id, err);
                Err(rpc_internal_error("remove_quote", err, &self.logger))
            }
        }
    }

    async fn live_updates_impl(
        req: LiveUpdatesRequest,
        mut responses: ServerStreamingSink<LiveUpdate>,
        mut msg_bus_rx: Receiver<Msg>,
    ) -> Result<(), grpcio::Error> {
        let filter_for_pair = {
            let pair = Pair::from(req.get_pair());
            if *pair.base_token_id == 0 && *pair.counter_token_id == 0 {
                None
            } else {
                Some(pair)
            }
        };

        while let Some(msg) = msg_bus_rx.recv().await {
            let mut live_update = LiveUpdate::default();
            match msg {
                Msg::SciQuoteAdded(quote) => {
                    if let Some(pair) = filter_for_pair {
                        if quote.pair() != &pair {
                            continue;
                        }
                    }

                    live_update.set_quote_added((&quote).into());
                }

                Msg::SciQuoteRemoved(quote) => {
                    if let Some(pair) = filter_for_pair {
                        if quote.pair() != &pair {
                            continue;
                        }
                    }

                    live_update.set_quote_removed(quote.id().into());
                }
            };
            responses.send((live_update, WriteFlags::default())).await?;
        }
        responses.close().await?;
        Ok(())
    }
}

impl<QB: QuoteBook> DeqsClientApi for ClientService<QB> {
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

    fn remove_quote(
        &mut self,
        ctx: RpcContext,
        req: RemoveQuoteRequest,
        sink: UnarySink<RemoveQuoteResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.remove_quote_impl(req, logger), logger)
        })
    }

    fn live_updates(
        &mut self,
        ctx: RpcContext<'_>,
        req: LiveUpdatesRequest,
        responses: ServerStreamingSink<LiveUpdate>,
    ) {
        let receiver = self.msg_bus_tx.subscribe();

        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            let logger = logger.clone();

            let future =
                Self::live_updates_impl(req, responses, receiver).map(move |future_result| {
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
    use deqs_quote_book_api::Quote;
    use deqs_quote_book_in_memory::InMemoryQuoteBook;
    use futures::{executor::block_on, StreamExt};
    use grpcio::{ChannelBuilder, EnvBuilder, Server, ServerBuilder, ServerCredentials};
    use mc_common::logger::test_with_logger;
    use mc_transaction_types::TokenId;
    use mc_util_grpc::ConnectionUriGrpcioChannel;
    use postage::broadcast;
    use rand::{rngs::StdRng, SeedableRng};
    use std::{
        str::FromStr,
        sync::{mpsc::channel, Arc},
        thread,
    };
    use tokio::select;

    fn create_test_client_and_server<QB: QuoteBook>(
        quote_book: &QB,
        logger: &Logger,
    ) -> (DeqsClientApiClient, Server, Receiver<Msg>) {
        let server_env = Arc::new(EnvBuilder::new().build());
        let (msg_bus_tx, msg_bus_rx) = broadcast::channel::<Msg>(1000);

        let notifying_quote_book =
            NotifyingQuoteBook::new(quote_book.clone(), Arc::new(Box::new(|_quote| {})));

        let client_service =
            ClientService::new(msg_bus_tx, notifying_quote_book, logger.clone()).into_service();
        let mut server = ServerBuilder::new(server_env)
            .register_service(client_service)
            .build()
            .unwrap();
        let port = server
            .add_listening_port("127.0.0.1:0", ServerCredentials::insecure())
            .expect("Could not create anonymous bind");
        server.start();

        let client_env = Arc::new(EnvBuilder::new().build());
        let ch = ChannelBuilder::default_channel_builder(client_env).connect_to_uri(
            &DeqsClientUri::from_str(&format!("insecure-deqs://127.0.0.1:{}", port)).unwrap(),
            logger,
        );
        let client_api = DeqsClientApiClient::new(ch);

        (client_api, server, msg_bus_rx)
    }

    #[test_with_logger]
    fn submit_quotes_add_quotes(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

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
        let mut quotes = resp
            .get_quotes()
            .iter()
            .map(|o| Quote::try_from(o).unwrap())
            .collect::<Vec<_>>();

        let mut quotes2 = quote_book
            .get_quotes(&pair, .., 0)
            .unwrap()
            .into_iter()
            .rev() // Quotes returned in newest to oldest quote
            .collect::<Vec<_>>();

        // Since quotes are added in parallel, the exact quote at which they get added
        // is not determinstic.
        quotes.sort();
        quotes2.sort();

        assert_eq!(quotes2, quotes);
    }

    #[test_with_logger]
    fn submit_quotes_refuses_identical_duplicate_request(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

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
            vec![QuoteStatusCode::QUOTE_ALREADY_EXISTS]
        );
    }

    #[test_with_logger]
    fn submit_quotes_doesnt_add_duplicate_quotes(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let sci1 = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let sci2 = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let sci3 = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci1).into(), (&sci2).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(
            resp.status_codes,
            vec![QuoteStatusCode::CREATED, QuoteStatusCode::CREATED]
        );

        let req = SubmitQuotesRequest {
            quotes: vec![(&sci1).into(), (&sci3).into(), (&sci2).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(
            resp.status_codes,
            vec![
                QuoteStatusCode::QUOTE_ALREADY_EXISTS,
                QuoteStatusCode::CREATED,
                QuoteStatusCode::QUOTE_ALREADY_EXISTS
            ]
        );
    }

    #[test_with_logger]
    fn get_quotes_filter_correctly(logger: Logger) {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let sci1 = create_sci(TokenId::from(1), TokenId::from(2), 10, 20, &mut rng);
        let sci2 = create_sci(TokenId::from(1), TokenId::from(2), 10, 50, &mut rng);
        let sci3 = create_sci(TokenId::from(3), TokenId::from(4), 10, 20, &mut rng);
        let sci4 = create_sci(TokenId::from(3), TokenId::from(4), 12, 50, &mut rng);

        let scis = [&sci1, &sci2, &sci3, &sci4]
            .into_iter()
            .map(mc_api::external::SignedContingentInput::from)
            .collect::<Vec<_>>();

        let req = SubmitQuotesRequest {
            quotes: scis.into(),
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
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3.clone(), sci4.clone()]);

        // Limit is respected
        req.set_limit(1);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3.clone()]);

        // Base range is respected
        req.set_limit(u64::MAX);
        req.set_base_range_max(10);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3.clone()]);

        req.set_base_range_min(11);
        req.set_base_range_max(12);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci4.clone()]);

        req.set_base_range_min(10);
        req.set_base_range_max(12);
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci3, sci4]);
    }

    #[test_with_logger]
    fn remove_quote_works(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let sci = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);

        // Quote is present
        let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes.len(), 1);

        // Remove it.
        let quote_id = deqs_api::deqs::QuoteId::from(quotes[0].id());
        let mut req = RemoveQuoteRequest::default();
        req.set_quote_id(quote_id);

        let resp = client_api.remove_quote(&req).expect("remove quote failed");
        assert_eq!(resp.get_quote(), &(&quotes[0]).into());

        // Try again, should fail.
        assert!(
            matches!(client_api.remove_quote(&req), Err(grpcio::Error::RpcFailure(status)) if status.code() == grpcio::RpcStatusCode::NOT_FOUND)
        );

        // Invalid quote id should fail.
        let req = RemoveQuoteRequest::default();
        assert!(
            matches!(client_api.remove_quote(&req), Err(grpcio::Error::RpcFailure(status)) if status.code() == grpcio::RpcStatusCode::INVALID_ARGUMENT)
        );
    }

    #[test_with_logger]
    fn streaming_works_without_filtering(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(0),
            counter_token_id: TokenId::from(1),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let (tx, rx) = channel();

        let thread_client_api = client_api.clone();
        let _join_handle = thread::spawn(move || {
            let req = LiveUpdatesRequest::default();
            let mut stream = thread_client_api
                .live_updates(&req)
                .expect("stream quotes failed");

            block_on(async {
                while let Some(resp) = stream.next().await {
                    match resp {
                        Ok(resp) => {
                            tx.send(resp).expect("send failed");
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            });
        });

        // Initially, the receiver should be empty.
        assert!(rx.try_recv().is_err());

        let sci = create_sci(pair.base_token_id, pair.counter_token_id, 10, 20, &mut rng);
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);
        let added_quote = &resp.get_quotes()[0];

        // We should now see our quote arrive in the stream.
        let resp = rx.recv().expect("recv failed");
        assert_eq!(resp.get_quote_added(), added_quote);

        // Remove the quote, we should see a live update.
        let mut req = RemoveQuoteRequest::default();
        req.set_quote_id(added_quote.get_id().clone());

        let _resp = client_api.remove_quote(&req).expect("remove quote failed");

        let resp = rx.recv().expect("recv failed");
        assert_eq!(resp.get_quote_removed(), added_quote.get_id());
    }

    #[test_with_logger]
    fn streaming_works_and_filters_correctly(logger: Logger) {
        let pair1 = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let pair2 = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(3),
        };

        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let (tx1, rx1) = channel();
        let (tx2, rx2) = channel();

        let thread_client_api = client_api.clone();
        let _join_handle = thread::spawn(move || {
            let mut req1 = LiveUpdatesRequest::default();
            req1.set_pair((&pair1).into()); // Filter only to pair1
            let mut stream1 = thread_client_api
                .live_updates(&req1)
                .expect("stream quotes failed");

            let mut req2 = LiveUpdatesRequest::default();
            req2.set_pair((&pair2).into()); // Filter only to pair2
            let mut stream2 = thread_client_api
                .live_updates(&req2)
                .expect("stream quotes failed");

            block_on(async {
                loop {
                    select! {
                        resp = stream1.next() => {
                            match resp {
                                Some(Ok(resp)) => {
                                    tx1.send(resp).expect("send failed");
                                }
                                _ => {
                                    break;
                                }
                            }
                        }
                        resp = stream2.next() => {
                            match resp {
                                Some(Ok(resp)) => {
                                    tx2.send(resp).expect("send failed");
                                }
                                _ => {
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        });

        // Initially, the receivers should be empty.
        assert!(rx1.try_recv().is_err());
        assert!(rx2.try_recv().is_err());

        let pair1_sci1 = create_sci(
            pair1.base_token_id,
            pair1.counter_token_id,
            10,
            20,
            &mut rng,
        );
        let pair1_sci2 = create_sci(
            pair1.base_token_id,
            pair1.counter_token_id,
            10,
            20,
            &mut rng,
        );
        let pair2_sci1 = create_sci(
            pair2.base_token_id,
            pair2.counter_token_id,
            10,
            20,
            &mut rng,
        );
        let pair2_sci2 = create_sci(
            pair2.base_token_id,
            pair2.counter_token_id,
            10,
            20,
            &mut rng,
        );
        let req = SubmitQuotesRequest {
            quotes: vec![
                (&pair1_sci1).into(),
                (&pair1_sci2).into(),
                (&pair2_sci1).into(),
                (&pair2_sci2).into(),
            ]
            .into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(
            resp.status_codes,
            vec![
                QuoteStatusCode::CREATED,
                QuoteStatusCode::CREATED,
                QuoteStatusCode::CREATED,
                QuoteStatusCode::CREATED
            ]
        );

        // We should now see our two pair1 quotes arrive in the first stream.
        let resp = rx1.recv().expect("recv failed");
        assert_eq!(resp.get_quote_added().get_pair(), &(&pair1).into());

        let resp = rx1.recv().expect("recv failed");
        assert_eq!(resp.get_quote_added().get_pair(), &(&pair1).into());

        // And pair2 quotes on the second stream
        let resp = rx2.recv().expect("recv failed");
        assert_eq!(resp.get_quote_added().get_pair(), &(&pair2).into());

        let resp = rx2.recv().expect("recv failed");
        assert_eq!(resp.get_quote_added().get_pair(), &(&pair2).into());
    }
}
