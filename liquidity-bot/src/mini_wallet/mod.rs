// Copyright (c) 2023 MobileCoin Inc.

mod account_ledger_scanner;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

// TODO
use account_ledger_scanner::AccountLedgerScanner;
use mc_account_keys::AccountKey;
use mc_blockchain_types::BlockIndex;
use mc_common::logger::Logger;
use mc_ledger_db::LedgerDB;
use mc_transaction_core::{ring_signature::KeyImage, tx::TxOut, Amount};
use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MatchedTxOut {
    tx_out: TxOut,
    amount: Amount,
    subaddress_index: u64,
    key_image: KeyImage,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct State {
    next_block_index: BlockIndex,
    matched_tx_outs: HashMap<KeyImage, MatchedTxOut>,
}
impl State {
    pub fn save(&self, path: &PathBuf) -> Result<(), Error> {
        let state = serde_json::to_string(self)?;
        std::fs::write(&path, state)?;
        Ok(())
    }
}

pub struct MiniWallet {
    state: Arc<Mutex<State>>,
    _account_ledger_scanner: AccountLedgerScanner,
}

impl MiniWallet {
    pub fn new(
        state_file: impl AsRef<Path>,
        ledger_db: LedgerDB,
        account_key: AccountKey,
        logger: Logger,
    ) -> Result<MiniWallet, Error> {
        let state_file = state_file.as_ref().to_path_buf();

        let state = if state_file.exists() {
            let state = std::fs::read_to_string(&state_file)?;
            let state: State = serde_json::from_str(&state)?;
            state
        } else {
            State::default()
        };

        let state = Arc::new(Mutex::new(state));

        let account_ledger_scanner =
            AccountLedgerScanner::new(ledger_db, account_key, state.clone(), state_file, logger);

        Ok(MiniWallet {
            state,
            _account_ledger_scanner: account_ledger_scanner,
        })
    }

    pub fn next_block_index(&self) -> BlockIndex {
        self.state.lock().unwrap().next_block_index
    }
}
