// Copyright (c) 2023 MobileCoin Inc.

use std::{collections::BTreeSet, str::FromStr, sync::Arc, time::Duration};

use deqs_api::{
    deqs::{RemoveQuoteRequest, SubmitQuotesRequest},
    deqs_grpc::DeqsClientApiClient,
    DeqsClientUri,
};
use deqs_quote_book::{InMemoryQuoteBook, Pair, QuoteBook, QuoteId, SynchronizedQuoteBook};
use deqs_server::Server;
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_account_keys::AccountKey;
use mc_common::logger::{async_test_with_logger, Logger};
use mc_ledger_db::{
    test_utils::{create_ledger, initialize_ledger},
    LedgerDB,
};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::{BlockVersion, TokenId};
use mc_util_grpc::ConnectionUriGrpcioChannel;
use rand::{rngs::StdRng, SeedableRng};
use tokio_retry::{strategy::FixedInterval, Retry};

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
    logger: &Logger,
) -> (TestServer, TestQuoteBook, DeqsClientApiClient) {
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book =
        SynchronizedQuoteBook::new(internal_quote_book, ledger_db.clone());

    for sci in initial_scis.into_iter() {
        synchronized_quote_book.add_sci(sci.clone(), None).unwrap();
    }

    let deqs_server = Server::start(
        synchronized_quote_book.clone(),
        DeqsClientUri::from_str("insecure-deqs://127.0.0.1:0/").unwrap(),
        p2p_bootstrap_from
            .into_iter()
            .map(|server| server.p2p_listen_addrs())
            .flatten()
            .collect(),
        None,
        None,
        None,
        logger.clone(),
    )
    .await
    .unwrap();

    let client_env = Arc::new(EnvBuilder::new().build());
    let ch = ChannelBuilder::default_channel_builder(client_env)
        .connect_to_uri(&deqs_server.grpc_listen_uri().unwrap(), &logger);
    let client_api = DeqsClientApiClient::new(ch);

    (deqs_server, synchronized_quote_book, client_api)
}

/// Test that two nodes propagate quotes being added and removed to eachother.
#[async_test_with_logger]
async fn e2e_two_nodes_quote_propagation(logger: Logger) {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let ledger_db = create_and_initialize_test_ledger();

    // Start two DEQS servers
    let (deqs_server1, quote_book1, client1) =
        start_deqs_server(&ledger_db, &[], &[], &logger).await;

    let (_deqs_server2, quote_book2, client2) =
        start_deqs_server(&ledger_db, &[&deqs_server1], &[], &logger).await;

    // Submit an SCI to the first server
    let sci =
        deqs_mc_test_utils::create_sci(TokenId::from(1), TokenId::from(2), 10000, 20000, &mut rng);

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

    // Request the second server to remove the SCI, the change should propagate to
    // the first server. First sanity test that SCI is in fact in the quote book
    // of the first server.
    assert_eq!(
        quote_book1.get_quote_by_id(&quote_id).unwrap().unwrap(),
        quote
    );

    let mut req = RemoveQuoteRequest::default();
    req.set_quote_id((&quote_id).into());
    let _resp = client2.remove_quote(&req).expect("remove quote failed");

    // Quote should eventually be removed from the first server
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
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
}

/// Test that two nodes exchange their quote books when they connect to
/// eachother.
#[async_test_with_logger]
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
            )
        })
        .collect::<Vec<_>>();

    // Start two DEQS servers
    let (deqs_server1, quote_book1, _client1) =
        start_deqs_server(&ledger_db, &[], &server1_scis, &logger).await;

    let (_deqs_server2, quote_book2, _client2) =
        start_deqs_server(&ledger_db, &[&deqs_server1], &server2_scis, &logger).await;

    // The combined set of quotes.
    let quotes1 = quote_book1.get_quotes(&pair, .., 0).unwrap();
    let quotes2 = quote_book2.get_quotes(&pair, .., 0).unwrap();
    let combined_quotes = BTreeSet::from_iter(quotes1.into_iter().chain(quotes2.into_iter()));
    assert_eq!(
        combined_quotes.len(),
        server1_scis.len() + server2_scis.len()
    );

    // The first server should eventually have all SCIs from the second server
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
    Retry::spawn(retry_strategy, || async {
        let quotes = BTreeSet::from_iter(quote_book1.get_quotes(&pair, .., 0).unwrap());
        if quotes == combined_quotes {
            Ok(())
        } else {
            Err(format!(
                "quotes: {}/{}",
                quotes.len(),
                combined_quotes.len()
            ))
        }
    })
    .await
    .unwrap();

    // The second server should eventually have all SCIs from the sfirst server
    let retry_strategy = FixedInterval::new(Duration::from_secs(1)).take(10); // limit to 10 retries
    Retry::spawn(retry_strategy, || async {
        let quotes = BTreeSet::from_iter(quote_book2.get_quotes(&pair, .., 0).unwrap());
        if quotes == combined_quotes {
            Ok(())
        } else {
            Err(format!(
                "quotes: {}/{}",
                quotes.len(),
                combined_quotes.len()
            ))
        }
    })
    .await
    .unwrap();
}
