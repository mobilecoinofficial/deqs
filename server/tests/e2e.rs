// Copyright (c) 2023 MobileCoin Inc.

use deqs_api::{
    deqs::{QuoteStatusCode, SubmitQuotesRequest},
    deqs_grpc::DeqsClientApiClient,
    DeqsClientUri,
};
use deqs_quote_book_api::{Pair, Quote, QuoteBook, QuoteId};
use deqs_quote_book_in_memory::InMemoryQuoteBook;
use deqs_quote_book_synchronized::SynchronizedQuoteBook;
use deqs_server::{Msg, Server};
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_account_keys::AccountKey;
use mc_common::logger::{async_test_with_logger, log, Logger};
use mc_fog_report_validation_test_utils::MockFogResolver;
use mc_ledger_db::{
    test_utils::{add_txos_and_key_images_to_ledger, create_ledger, initialize_ledger},
    LedgerDB,
};
use mc_transaction_builder::test_utils::get_transaction;
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::{BlockVersion, TokenId};
use mc_util_grpc::ConnectionUriGrpcioChannel;
use postage::broadcast;
use rand::{rngs::StdRng, SeedableRng};
use std::{collections::BTreeSet, str::FromStr, sync::Arc, time::Duration};
use tokio_retry::{strategy::FixedInterval, Retry};

/// Maximum number of messages that can be queued in the message bus.
const MSG_BUS_QUEUE_SIZE: usize = 1000;

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

// Helper to start a deqs server.
type TestQuoteBook = SynchronizedQuoteBook<InMemoryQuoteBook, LedgerDB>;
type TestServer = Server<TestQuoteBook>;
async fn start_deqs_server(
    ledger_db: &LedgerDB,
    p2p_bootstrap_from: &[&TestServer],
    initial_scis: &[SignedContingentInput],
    quote_minimum_map: Vec<(TokenId, u64)>,
    logger: &Logger,
) -> (TestServer, TestQuoteBook, DeqsClientApiClient) {
    let (msg_bus_tx, msg_bus_rx) = broadcast::channel::<Msg>(MSG_BUS_QUEUE_SIZE);
    let remove_quote_callback = TestServer::get_remove_quote_callback_function(msg_bus_tx.clone());
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(
        internal_quote_book,
        ledger_db.clone(),
        remove_quote_callback,
        logger.clone(),
    );

    let (deqs_server, client_api) = start_deqs_server_for_quotebook(
        initial_scis,
        &synchronized_quote_book,
        p2p_bootstrap_from,
        msg_bus_tx,
        msg_bus_rx,
        quote_minimum_map,
        logger,
    )
    .await;

    (deqs_server, synchronized_quote_book, client_api)
}

async fn start_deqs_server_for_quotebook<Q: QuoteBook>(
    initial_scis: &[SignedContingentInput],
    quote_book: &Q,
    p2p_bootstrap_from: &[&TestServer],
    msg_bus_tx: broadcast::Sender<Msg>,
    msg_bus_rx: broadcast::Receiver<Msg>,
    quote_minimum_map: Vec<(TokenId, u64)>,
    logger: &Logger,
) -> (Server<Q>, DeqsClientApiClient) {
    for sci in initial_scis.iter() {
        quote_book.add_sci(sci.clone(), None).unwrap();
    }

    let deqs_server = Server::start(
        quote_book.clone(),
        DeqsClientUri::from_str("insecure-deqs://127.0.0.1:0/").unwrap(),
        p2p_bootstrap_from
            .iter()
            .flat_map(|server| server.p2p_listen_addrs())
            .collect(),
        None,
        None,
        None,
        msg_bus_tx,
        msg_bus_rx,
        quote_minimum_map,
        logger.clone(),
    )
    .await
    .unwrap();

    let client_env = Arc::new(EnvBuilder::new().build());
    let ch = ChannelBuilder::default_channel_builder(client_env)
        .connect_to_uri(&deqs_server.grpc_listen_uri().unwrap(), logger);
    let client_api = DeqsClientApiClient::new(ch);
    (deqs_server, client_api)
}

// Helper to wait until a quote book has a set of quotes, or timeout.
async fn wait_for_quotes(
    pair: &Pair,
    quote_book: &TestQuoteBook,
    expected_quotes: &BTreeSet<Quote>,
) {
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(60); // limit to 60 retries
    Retry::spawn(retry_strategy, || async {
        let quotes = BTreeSet::from_iter(quote_book.get_quotes(pair, .., 0).unwrap());
        if &quotes == expected_quotes {
            Ok(())
        } else {
            Err(format!(
                "quotes: {}/{}",
                quotes.len(),
                expected_quotes.len()
            ))
        }
    })
    .await
    .unwrap();
}

/// Test that two nodes propagate quotes being added and removed to eachother.
#[async_test_with_logger(flavor = "multi_thread")]
async fn e2e_two_nodes_quote_propagation(logger: Logger) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let ledger_db = create_and_initialize_test_ledger();

    // Start two DEQS servers
    let (deqs_server1, _quote_book1, client1) =
        start_deqs_server(&ledger_db, &[], &[], vec![], &logger).await;

    let (_deqs_server2, quote_book2, _client2) =
        start_deqs_server(&ledger_db, &[&deqs_server1], &[], vec![], &logger).await;

    // Submit an SCI to the first server
    let sci = deqs_mc_test_utils::create_sci(
        TokenId::from(1),
        TokenId::from(2),
        10000,
        20000,
        &mut rng,
        Some(&ledger_db),
    );

    let req = SubmitQuotesRequest {
        quotes: vec![(&sci).into()].into(),
        ..Default::default()
    };
    let resp = client1.submit_quotes(&req).expect("submit quote failed");
    let quote_id = QuoteId::try_from(resp.get_quotes()[0].get_id()).unwrap();

    // After a few moments the second server should have the SCI in its quote book
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
    let quote = Retry::spawn(retry_strategy, || async {
        let quote = quote_book2.get_quote_by_id(&quote_id);
        match quote {
            Ok(Some(quote)) => Ok(quote),
            Ok(None) => Err("not yet".to_string()),
            Err(e) => Err(format!("error: {:?}", e)),
        }
    })
    .await
    .unwrap();

    assert_eq!(quote.sci(), &sci);
}

/// Test that nodes don't accept dust quotes directly, but will propagate dust
/// quotes to each other.
#[async_test_with_logger(flavor = "multi_thread")]
async fn e2e_two_nodes_dust_propagation(logger: Logger) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let ledger_db = create_and_initialize_test_ledger();

    // Start two DEQS servers
    let (deqs_server1, quote_book1, client1) = start_deqs_server(
        &ledger_db,
        &[],
        &[],
        vec![(TokenId::from(1), 1000)],
        &logger,
    )
    .await;

    let (_deqs_server2, _quote_book2, client2) = start_deqs_server(
        &ledger_db,
        &[&deqs_server1],
        &[],
        vec![(TokenId::from(1), 100)],
        &logger,
    )
    .await;

    // Submit an SCI to the first server
    let dust_sci = deqs_mc_test_utils::create_sci(
        TokenId::from(1),
        TokenId::from(2),
        500,
        20000,
        &mut rng,
        Some(&ledger_db),
    );

    let req = SubmitQuotesRequest {
        quotes: vec![(&dust_sci).into()].into(),
        ..Default::default()
    };

    // Submitting to the first server should fail because it's smaller than its dust
    // level.
    let resp = client1.submit_quotes(&req).expect("submit quote failed");
    assert_eq!(
        resp.get_status_codes(),
        vec![QuoteStatusCode::UNSUPPORTED_SCI]
    );
    assert_eq!(
        resp.get_error_messages(),
        vec!["Unsupported SCI: Quote volume is too small for deqs. Quotes with base_token: 1 require a minimum of: 1000"]
    );

    // Submitting to the second server should succeed because it's bigger than its
    // dust level.
    let resp = client2.submit_quotes(&req).expect("submit quote failed");
    let quote_id = QuoteId::try_from(resp.get_quotes()[0].get_id()).unwrap();

    // After a few moments the first server should have the SCI in its quote book
    // since peer to peer quotes are exempt from the dust check.
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
    let quote = Retry::spawn(retry_strategy, || async {
        let quote = quote_book1.get_quote_by_id(&quote_id);
        match quote {
            Ok(Some(quote)) => Ok(quote),
            Ok(None) => Err("not yet".to_string()),
            Err(e) => Err(format!("error: {:?}", e)),
        }
    })
    .await
    .unwrap();

    assert_eq!(quote.sci(), &dust_sci);
}

/// Test that two nodes propagate quotes being added, and when the ledger
/// invalidates it, they both remove it
#[async_test_with_logger(flavor = "multi_thread")]
async fn e2e_two_nodes_quote_propagation_and_removal(logger: Logger) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let mut ledger_db = create_and_initialize_test_ledger();

    // Start two DEQS servers
    let (deqs_server1, quote_book1, client1) =
        start_deqs_server(&ledger_db, &[], &[], vec![], &logger).await;

    let (_deqs_server2, quote_book2, _client2) =
        start_deqs_server(&ledger_db, &[&deqs_server1], &[], vec![], &logger).await;

    // Submit an SCI to the first server
    let sci = deqs_mc_test_utils::create_sci(
        TokenId::from(1),
        TokenId::from(2),
        10000,
        20000,
        &mut rng,
        Some(&ledger_db),
    );

    let req = SubmitQuotesRequest {
        quotes: vec![(&sci).into()].into(),
        ..Default::default()
    };
    let resp = client1.submit_quotes(&req).expect("submit quote failed");
    let quote_id = QuoteId::try_from(resp.get_quotes()[0].get_id()).unwrap();

    // After a few moments the second server should have the SCI in its quote book
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
    let quote = Retry::spawn(retry_strategy, || async {
        let quote = quote_book2.get_quote_by_id(&quote_id);
        match quote {
            Ok(Some(quote)) => Ok(quote),
            Ok(None) => Err("not yet".to_string()),
            Err(e) => Err(format!("error: {:?}", e)),
        }
    })
    .await
    .unwrap();

    assert_eq!(quote.sci(), &sci);

    // Add the quote to the ledger for the first quotebook. The quote should
    // eventually be removed from the second quotebook
    assert_eq!(
        quote_book1.get_quote_by_id(&quote_id).unwrap().unwrap(),
        quote
    );
    let key_image = quote.sci().key_image();

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
        &mut ledger_db,
        BlockVersion::MAX,
        tx.prefix.outputs,
        vec![key_image],
        &mut rng,
    )
    .unwrap();

    // Quote should eventually be removed from the first server
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(30); // limit to 30 retries
    Retry::spawn(retry_strategy, || async {
        let quote = quote_book1.get_quote_by_id(&quote_id);
        match quote {
            Ok(Some(_quote)) => Err("not yet".to_string()),
            Ok(None) => Ok(()),
            Err(e) => Err(format!("error: {:?}", e)),
        }
    })
    .await
    .unwrap();

    // Quote should eventually be removed from the second server
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(30); // limit to 30 retries
    Retry::spawn(retry_strategy, || async {
        let quote = quote_book2.get_quote_by_id(&quote_id);
        match quote {
            Ok(Some(_quote)) => Err("not yet".to_string()),
            Ok(None) => Ok(()),
            Err(e) => Err(format!("error: {:?}", e)),
        }
    })
    .await
    .unwrap();
}

/// Test that two nodes exchange their quote books when they connect to
/// eachother.
#[async_test_with_logger(flavor = "multi_thread")]
async fn e2e_two_nodes_initial_sync(logger: Logger) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let ledger_db = create_and_initialize_test_ledger();

    let pair = Pair {
        base_token_id: TokenId::from(1),
        counter_token_id: TokenId::from(2),
    };

    let server1_scis = (0..10)
        .map(|i| {
            deqs_mc_test_utils::create_sci(
                pair.base_token_id,
                pair.counter_token_id,
                10000 * i,
                20000,
                &mut rng,
                Some(&ledger_db),
            )
        })
        .collect::<Vec<_>>();

    let server2_scis = (0..20)
        .map(|i| {
            deqs_mc_test_utils::create_sci(
                pair.base_token_id,
                pair.counter_token_id,
                10000,
                20000 * i,
                &mut rng,
                Some(&ledger_db),
            )
        })
        .collect::<Vec<_>>();

    // Start two DEQS servers
    let (deqs_server1, quote_book1, _client1) =
        start_deqs_server(&ledger_db, &[], &server1_scis, vec![], &logger).await;

    let (_deqs_server2, quote_book2, _client2) =
        start_deqs_server(&ledger_db, &[&deqs_server1], &server2_scis, vec![], &logger).await;

    // The combined set of quotes.
    let quotes1 = quote_book1.get_quotes(&pair, .., 0).unwrap();
    let quotes2 = quote_book2.get_quotes(&pair, .., 0).unwrap();
    let combined_quotes = BTreeSet::from_iter(quotes1.into_iter().chain(quotes2.into_iter()));
    assert_eq!(
        combined_quotes.len(),
        server1_scis.len() + server2_scis.len()
    );

    // The first server should eventually have all SCIs from the second server
    wait_for_quotes(&pair, &quote_book1, &combined_quotes).await;

    // The second server should eventually have all SCIs from the sfirst server
    wait_for_quotes(&pair, &quote_book2, &combined_quotes).await;
}

/// Test that a five node network propagates quotes correctly. This is a
/// combination of the two tests above, with a few more nodes.
#[async_test_with_logger(flavor = "multi_thread")]
async fn e2e_multiple_nodes_play_nicely(logger: Logger) {
    const NUM_NODES: usize = 5;
    let pair = Pair {
        base_token_id: TokenId::from(1),
        counter_token_id: TokenId::from(2),
    };

    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let ledger_db = create_and_initialize_test_ledger();

    let mut last_server = None;
    let mut servers = Vec::new();
    let mut clients = Vec::new();
    let mut quote_books = Vec::new();
    let mut quotes = Vec::new();
    for _ in 0..NUM_NODES {
        let scis = (0..10)
            .map(|i| {
                deqs_mc_test_utils::create_sci(
                    pair.base_token_id,
                    pair.counter_token_id,
                    10000 * i,
                    20000,
                    &mut rng,
                    Some(&ledger_db),
                )
            })
            .collect::<Vec<_>>();

        let bootstrap_peers = last_server
            .as_ref()
            .map(|server| vec![server])
            .unwrap_or_default();

        let (server, quote_book, client) =
            start_deqs_server(&ledger_db, &bootstrap_peers, &scis, vec![], &logger).await;

        if let Some(prev_server) = last_server.take() {
            servers.push(prev_server);
        }
        last_server = Some(server);

        clients.push(client);
        quotes.push(quote_book.get_quotes(&pair, .., 0).unwrap());
        quote_books.push(quote_book);
    }
    servers.push(last_server.unwrap());

    // Test that all servers got all initial quotes.
    let combined_quotes = BTreeSet::from_iter(quotes.into_iter().flatten());

    for (i, quote_book) in quote_books.iter().enumerate() {
        log::info!(logger, "Waiting for quotes on server {}", i);
        wait_for_quotes(&pair, quote_book, &combined_quotes).await;
    }

    // Add a quote to one of the servers and see that all servers eventually see
    // it.
    let new_sci = deqs_mc_test_utils::create_sci(
        pair.base_token_id,
        pair.counter_token_id,
        1234,
        20000,
        &mut rng,
        Some(&ledger_db),
    );

    let req = SubmitQuotesRequest {
        quotes: vec![(&new_sci).into()].into(),
        ..Default::default()
    };
    let resp = clients[3].submit_quotes(&req).expect("submit quote failed");
    let quote_id = QuoteId::try_from(resp.get_quotes()[0].get_id()).unwrap();

    // All servers should now be aware of this quote.
    for (i, quote_book) in quote_books.iter().enumerate() {
        log::info!(logger, "Waiting for quote {} on server {}", quote_id, i);

        let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
        let quote = Retry::spawn(retry_strategy, || async {
            let quote = quote_book.get_quote_by_id(&quote_id);
            match quote {
                Ok(Some(quote)) => Ok(quote),
                Ok(None) => Err("not yet".to_string()),
                Err(e) => Err(format!("error: {:?}", e)),
            }
        })
        .await
        .unwrap();

        assert_eq!(quote.sci(), &new_sci);
    }
}
