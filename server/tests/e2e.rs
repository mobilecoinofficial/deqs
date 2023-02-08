// Copyright (c) 2023 MobileCoin Inc.

use std::str::FromStr;

use deqs_api::DeqsClientUri;
use deqs_quote_book::{InMemoryQuoteBook, SynchronizedQuoteBook};
use deqs_server::Server;
use mc_account_keys::AccountKey;
use mc_common::logger::{async_test_with_logger, Logger};
use mc_ledger_db::{
    test_utils::{create_ledger, initialize_ledger},
    LedgerDB,
};
use mc_transaction_types::BlockVersion;
use rand::{rngs::StdRng, SeedableRng};

fn create_and_initialize_test_ledger() -> LedgerDB {
    //Create a ledger_db
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
    let block_version = BlockVersion::MAX;
    let sender = AccountKey::random(&mut rng);
    let mut ledger = create_ledger();

    //Initialize that db
    let n_blocks = 3;
    initialize_ledger(block_version, &mut ledger, n_blocks, &sender, &mut rng);

    ledger
}

#[async_test_with_logger]
async fn e2e(logger: Logger) {
    let ledger_db = create_and_initialize_test_ledger();
    let internal_quote_book = InMemoryQuoteBook::default();
    let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger_db);

    let deqs_server = Server::start(
        synchronized_quote_book,
        DeqsClientUri::from_str("insecure-deqs://127.0.0.1:0/").unwrap(),
        vec![],
        None,
        None,
        None,
        logger.clone(),
    )
    .await
    .unwrap();

    panic!(
        "{:?} {:?}",
        deqs_server.grpc_listen_uri(),
        deqs_server.p2p_listen_addrs(),
    );
}
