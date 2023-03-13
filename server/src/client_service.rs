// Copyright (c) 2023 MobileCoin Inc.

use crate::{Msg, SVC_COUNTERS};
use deqs_api::{
    deqs::{
        GetQuotesRequest, GetQuotesResponse, LiveUpdate, LiveUpdatesRequest, QuoteStatusCode,
        SubmitQuotesRequest, SubmitQuotesResponse,
    },
    deqs_grpc::{create_deqs_client_api, DeqsClientApi},
};
use deqs_quote_book_api::{Error as QuoteBookError, Pair, QuoteBook};
use futures::{FutureExt, SinkExt};
use grpcio::{RpcContext, RpcStatus, ServerStreamingSink, Service, UnarySink, WriteFlags};
use mc_common::logger::{log, scoped_global_logger, Logger};
use mc_transaction_extra::SignedContingentInput;
use mc_util_grpc::{
    rpc_internal_error, rpc_invalid_arg_error, rpc_logger, rpc_unavailable_error, send_result,
};
use postage::{
    broadcast::{Receiver, Sender},
    prelude::Stream,
    sink::Sink,
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// Maximum number of pending quotes for deqs before rejecting
/// submit quote requests.
const PENDING_LIMIT: usize = 30;

/// A function for validating that scis should be accepted by the server. It
/// returns Ok if it is valid, and otherwise returns an error. It receives
/// 1 argument:
/// - A Sci to be validated
pub type SciValidator =
    Arc<dyn Fn(&SignedContingentInput) -> Result<(), QuoteBookError> + Sync + Send>;

/// GRPC Client service
#[derive(Clone)]
pub struct ClientService<OB: QuoteBook> {
    /// Message bus sender.
    msg_bus_tx: Sender<Msg>,

    /// Quote book.
    quote_book: OB,

    /// Validates Scis for specific server config before passing it to the
    /// quotebook
    sci_validator: SciValidator,

    /// The number of quotes that are still in progress
    pending_quotes: Arc<AtomicUsize>,

    /// Logger.
    logger: Logger,
}

impl<OB: QuoteBook> ClientService<OB> {
    /// Create a new ClientService
    pub fn new(
        msg_bus_tx: Sender<Msg>,
        quote_book: OB,
        sci_validator: SciValidator,
        logger: Logger,
    ) -> Self {
        Self {
            msg_bus_tx,
            quote_book,
            sci_validator,
            pending_quotes: Arc::new(AtomicUsize::new(0)),
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
            .map(|sci| {
                // Validate sci before trying to add it to the quote book.
                (self.sci_validator)(&sci)?;
                self.quote_book.add_sci(sci, Some(timestamp))
            })
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
                Err(ref err @ QuoteBookError::QuoteAlreadyExists { ref existing_quote }) => {
                    error_messages.push(err.to_string());
                    status_codes.push(err.into());
                    quotes.push((&**existing_quote).into());
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

impl<OB: QuoteBook> DeqsClientApi for ClientService<OB> {
    fn submit_quotes(
        &mut self,
        ctx: RpcContext,
        req: SubmitQuotesRequest,
        sink: UnarySink<SubmitQuotesResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            let result: Result<SubmitQuotesResponse, RpcStatus> =
                if req.quotes.len() >= PENDING_LIMIT {
                    Err(rpc_invalid_arg_error(
                        "Too many quotes",
                        QuoteBookError::ImplementationSpecific(
                            "Too many quotes in one request".to_owned(),
                        ),
                        logger,
                    ))
                } else {
                    let num_quotes = req.quotes.len();
                    self.pending_quotes.fetch_add(num_quotes, Ordering::SeqCst);
                    if (self.pending_quotes.load(Ordering::SeqCst)) >= PENDING_LIMIT {
                        // This node is over capacity, and is not accepting proposed quotes.
                        self.pending_quotes.fetch_sub(num_quotes, Ordering::SeqCst);
                        Err(rpc_unavailable_error(
                            "Server over capacity",
                            QuoteBookError::ImplementationSpecific(
                                "Server is over capacity".to_owned(),
                            ),
                            logger,
                        ))
                    } else {
                        let result = self.submit_quotes_impl(req, logger);
                        self.pending_quotes.fetch_sub(num_quotes, Ordering::SeqCst);
                        result
                    }
                };
            send_result(ctx, sink, result, logger)
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
    use deqs_api::{deqs::LiveUpdate, deqs_grpc::DeqsClientApiClient, DeqsClientUri};
    use deqs_mc_test_utils::create_sci;
    use deqs_quote_book_api::Quote;
    use deqs_quote_book_in_memory::InMemoryQuoteBook;
    use deqs_quote_book_synchronized::SynchronizedQuoteBook;
    use futures::{executor::block_on, StreamExt};
    use grpcio::{ChannelBuilder, EnvBuilder, Server, ServerBuilder, ServerCredentials};
    use mc_account_keys::AccountKey;
    use mc_common::logger::{async_test_with_logger, test_with_logger};
    use mc_fog_report_validation_test_utils::MockFogResolver;
    use mc_ledger_db::{
        test_utils::{add_txos_and_key_images_to_ledger, create_ledger, initialize_ledger},
        Ledger, LedgerDB,
    };
    use mc_transaction_builder::test_utils::get_transaction;
    use mc_transaction_types::{BlockVersion, TokenId};
    use mc_util_grpc::ConnectionUriGrpcioChannel;
    use postage::broadcast;
    use prometheus::core::{Atomic, AtomicU64};
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
    ) -> (DeqsClientApiClient, Server, Sender<Msg>, Receiver<Msg>) {
        let server_env = Arc::new(EnvBuilder::new().build());
        let (msg_bus_tx, msg_bus_rx) = broadcast::channel::<Msg>(1000);
        let sci_validator: SciValidator = Arc::new(|sci| {
            let base_value = sci.pseudo_output_amount.value;
            if base_value < 3 {
                return Err(QuoteBookError::UnsupportedSci(
                    "Quote is too small for deqs".to_owned(),
                ));
            }
            Ok(())
        });

        let client_service = ClientService::new(
            msg_bus_tx.clone(),
            quote_book.clone(),
            sci_validator,
            logger.clone(),
        )
        .into_service();
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
            &DeqsClientUri::from_str(&format!("insecure-deqs://127.0.0.1:{port}")).unwrap(),
            logger,
        );
        let client_api = DeqsClientApiClient::new(ch);

        (client_api, server, msg_bus_tx, msg_bus_rx)
    }

    fn create_live_updates_subscriber(
        client_api: DeqsClientApiClient,
        logger: Logger,
    ) -> (
        std::sync::mpsc::Receiver<LiveUpdate>,
        std::thread::JoinHandle<()>,
    ) {
        let (live_updates_tx, live_updates_rx) = channel();

        let join_handle = thread::spawn(move || {
            let req = LiveUpdatesRequest::default();
            let mut stream = client_api.live_updates(&req).expect("stream quotes failed");

            block_on(async {
                while let Some(resp) = stream.next().await {
                    match resp {
                        Ok(resp) => {
                            log::debug!(logger, "Got a live update: {:?}", resp);
                            live_updates_tx.send(resp).expect("send failed");
                            log::debug!(logger, "Sent live update on");
                        }
                        Err(err) => {
                            log::info!(logger, "Live updates thread exiting: {}", err);
                            break;
                        }
                    }
                }
            });
        });

        (live_updates_rx, join_handle)
    }

    fn create_and_initialize_test_ledger() -> LedgerDB {
        // Create a ledger_db
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let block_version = BlockVersion::MAX;
        let sender = AccountKey::random(&mut rng);
        let mut ledger = create_ledger();

        // Initialize that db
        let n_blocks = 3;
        initialize_ledger(block_version, &mut ledger, n_blocks, &sender, &mut rng);

        ledger
    }

    #[test_with_logger]
    fn submit_quotes_add_quotes(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let scis = (0..10)
            .map(|_| {
                create_sci(
                    pair.base_token_id,
                    pair.counter_token_id,
                    10,
                    20,
                    &mut rng,
                    None,
                )
            })
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

    #[async_test_with_logger(flavor = "multi_thread")]
    async fn submit_quotes_rejects_quotes_above_the_pending_limit(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        // Submitting SCIs under the pending limit should be successful
        let scis = (0..10)
            .map(|_| {
                create_sci(
                    pair.base_token_id,
                    pair.counter_token_id,
                    10,
                    20,
                    &mut rng,
                    None,
                )
            })
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

        // Submitting SCIs above the pending limit should fail
        let too_many_scis_at_once = (0..PENDING_LIMIT)
            .map(|_| {
                create_sci(
                    pair.base_token_id,
                    pair.counter_token_id,
                    10,
                    20,
                    &mut rng,
                    None,
                )
            })
            .map(|sci| mc_api::external::SignedContingentInput::from(&sci))
            .collect::<Vec<_>>();

        let too_many_scis_at_once_req = SubmitQuotesRequest {
            quotes: too_many_scis_at_once.clone().into(),
            ..Default::default()
        };
        client_api
            .submit_quotes(&too_many_scis_at_once_req)
            .expect_err("Submitting too many requests should fail");
        let (left, right) = too_many_scis_at_once.split_at(PENDING_LIMIT / 2);

        let left_req = SubmitQuotesRequest {
            quotes: left.clone().into(),
            ..Default::default()
        };
        let right_req = SubmitQuotesRequest {
            quotes: right.clone().into(),
            ..Default::default()
        };
        let left_client = client_api.clone();
        let right_client = client_api.clone();
        let successes = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let left_successes = successes.clone();
        let left_failures = failures.clone();
        let right_successes = successes.clone();
        let right_failures = failures.clone();
        let mut handles = vec![];
        handles.push(tokio::spawn(async move {
            let result = left_client.submit_quotes(&left_req);
            if result.is_ok() {
                left_successes.inc_by(1);
            } else {
                left_failures.inc_by(1);
            }
        }));
        handles.push(tokio::spawn(async move {
            let result = right_client.submit_quotes(&right_req);
            if result.is_ok() {
                right_successes.inc_by(1);
            } else {
                right_failures.inc_by(1);
            }
        }));
        futures::future::join_all(handles).await;
        assert_eq!(successes.get(), 1);
        assert_eq!(failures.get(), 1);
    }

    #[test_with_logger]
    fn submit_quotes_refuses_dust(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        // Dust sized quotes should be rejected
        let scis = (0..10)
            .map(|_| {
                create_sci(
                    pair.base_token_id,
                    pair.counter_token_id,
                    1,
                    20,
                    &mut rng,
                    None,
                )
            })
            .map(|sci| mc_api::external::SignedContingentInput::from(&sci))
            .collect::<Vec<_>>();

        let req = SubmitQuotesRequest {
            quotes: scis.clone().into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");

        assert_eq!(
            resp.get_status_codes(),
            vec![QuoteStatusCode::UNSUPPORTED_SCI; scis.len()]
        );
        assert_eq!(
            resp.get_error_messages(),
            vec!["Unsupported SCI: Quote is too small for deqs"; scis.len()]
        );

        // Larger than dust quotes should be accepted
        let scis = (0..10)
            .map(|_| {
                create_sci(
                    pair.base_token_id,
                    pair.counter_token_id,
                    10,
                    20,
                    &mut rng,
                    None,
                )
            })
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
    }

    #[test_with_logger]
    fn submit_quotes_refuses_identical_duplicate_request(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let sci = create_sci(
            pair.base_token_id,
            pair.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);
        let quote = Quote::try_from(&resp.quotes[0]).unwrap();

        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(
            resp.status_codes,
            vec![QuoteStatusCode::QUOTE_ALREADY_EXISTS]
        );
        let quote2 = Quote::try_from(&resp.quotes[0]).unwrap();

        assert_eq!(quote, quote2);
    }

    #[test_with_logger]
    fn submit_quotes_doesnt_add_duplicate_quotes(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(1),
            counter_token_id: TokenId::from(2),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let sci1 = create_sci(
            pair.base_token_id,
            pair.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let sci2 = create_sci(
            pair.base_token_id,
            pair.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let sci3 = create_sci(
            pair.base_token_id,
            pair.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci1).into(), (&sci2).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(
            resp.status_codes,
            vec![QuoteStatusCode::CREATED, QuoteStatusCode::CREATED]
        );
        let quote1 = Quote::try_from(&resp.quotes[0]).unwrap();
        let quote2 = Quote::try_from(&resp.quotes[1]).unwrap();

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

        assert_eq!(quote1, Quote::try_from(&resp.quotes[0]).unwrap());
        assert_eq!(quote2, Quote::try_from(&resp.quotes[2]).unwrap());
    }

    #[test_with_logger]
    fn get_quotes_filter_correctly(logger: Logger) {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let sci1 = create_sci(TokenId::from(1), TokenId::from(2), 10, 20, &mut rng, None);
        let sci2 = create_sci(TokenId::from(1), TokenId::from(2), 10, 50, &mut rng, None);
        let sci3 = create_sci(TokenId::from(3), TokenId::from(4), 10, 20, &mut rng, None);
        let sci4 = create_sci(TokenId::from(3), TokenId::from(4), 12, 50, &mut rng, None);

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
    fn streaming_works_without_filtering(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(0),
            counter_token_id: TokenId::from(1),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let quote_book = InMemoryQuoteBook::default();
        let (client_api, _server, mut msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        let (live_updates_rx, _join_handle) =
            create_live_updates_subscriber(client_api.clone(), logger.clone());

        // Initially, the receiver should be empty.
        assert!(live_updates_rx.try_recv().is_err());

        let sci = create_sci(
            pair.base_token_id,
            pair.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);

        // Quote is present
        let quotes = quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes.len(), 1);

        let added_quote = &resp.get_quotes()[0];

        // We should now see our quote arrive in the stream.
        let resp = live_updates_rx.recv().expect("recv failed");
        assert_eq!(resp.get_quote_added(), added_quote);

        // There should not be any more updates right now
        assert!(live_updates_rx.try_recv().is_err());

        // Announce over the message bus that a quote has been removed
        msg_bus_tx
            .blocking_send(Msg::SciQuoteRemoved(added_quote.try_into().unwrap()))
            .expect("msg_bus reader died");

        // The live update subscription should get an update
        let resp = live_updates_rx.recv().expect("recv failed");
        log::debug!(logger, "live_updates_rx.recv() returned");
        assert_eq!(resp.get_quote_removed(), added_quote.get_id());
    }

    #[test_with_logger]
    fn submitting_expired_quotes_produces_quote_is_stale_error(logger: Logger) {
        let pair = Pair {
            base_token_id: TokenId::from(0),
            counter_token_id: TokenId::from(1),
        };
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

        let mut ledger = create_and_initialize_test_ledger();
        let starting_blocks = ledger.num_blocks().unwrap();

        // HACK: We aren't using the live updates in this particular test,
        // so we don't need to put a proper remove_quote_callback here.
        let quote_book = SynchronizedQuoteBook::new(
            InMemoryQuoteBook::default(),
            ledger.clone(),
            Box::new(|_| {}),
            logger.clone(),
        );

        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
            create_test_client_and_server(&quote_book, &logger);

        // Make and submit an sci
        let sci = create_sci(
            pair.base_token_id,
            pair.counter_token_id,
            10,
            20,
            &mut rng,
            Some(&ledger),
        );
        let key_image = sci.key_image();
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        // Number of blocks has gone up because the Txos being spent by the sci were
        // added to the ledger
        assert_eq!(ledger.num_blocks().unwrap(), starting_blocks + 1);
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::CREATED]);

        // We should see this sci if we call get quotes
        let mut req = GetQuotesRequest::default();
        req.set_pair((&pair).into());
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![sci.clone()]);

        // Build a Tx and add its outputs, and the sci's key image, to the ledger
        let block_version = BlockVersion::MAX;
        let fog_resolver = MockFogResolver::default();

        let offerer_account = AccountKey::random(&mut rng);

        let tx = get_transaction(
            block_version,
            TokenId::from(0),
            2,
            2,
            &offerer_account,
            &offerer_account,
            fog_resolver,
            &mut rng,
        )
        .unwrap();
        add_txos_and_key_images_to_ledger(
            &mut ledger,
            BlockVersion::MAX,
            tx.prefix.outputs,
            vec![key_image],
            &mut rng,
        )
        .unwrap();

        assert_eq!(ledger.num_blocks().unwrap(), starting_blocks + 2);

        // Block until we have processed this block
        let mut tries = 500usize;
        while quote_book.get_current_block_index() + 1 < ledger.num_blocks().unwrap() {
            std::thread::sleep(std::time::Duration::from_millis(100));
            tries -= 1;
            assert_ne!(tries, 0, "Quote book failed to catch up in time");
        }

        // We should see an empty book now if we call get quotes
        let mut req = GetQuotesRequest::default();
        req.set_pair((&pair).into());
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![]);

        // If we try to add the same quote back, it should be rejected as stale
        let req = SubmitQuotesRequest {
            quotes: vec![(&sci).into()].into(),
            ..Default::default()
        };
        let resp = client_api.submit_quotes(&req).expect("submit quote failed");
        assert_eq!(resp.status_codes, vec![QuoteStatusCode::QUOTE_IS_STALE]);

        // The quote book should still be empty
        let mut req = GetQuotesRequest::default();
        req.set_pair((&pair).into());
        let resp = client_api.get_quotes(&req).expect("get quotes failed");
        let received_scis = resp
            .get_quotes()
            .iter()
            .map(|quote| SignedContingentInput::try_from(quote.get_sci()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(received_scis, vec![]);
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
        let (client_api, _server, _msg_bus_tx, _msg_bus_rx) =
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
            None,
        );
        let pair1_sci2 = create_sci(
            pair1.base_token_id,
            pair1.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let pair2_sci1 = create_sci(
            pair2.base_token_id,
            pair2.counter_token_id,
            10,
            20,
            &mut rng,
            None,
        );
        let pair2_sci2 = create_sci(
            pair2.base_token_id,
            pair2.counter_token_id,
            10,
            20,
            &mut rng,
            None,
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
