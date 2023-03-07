// Copyright (c) 2023 MobileCoin Inc.

//! A minimalistic "wallet" - a ledger scanner that tracks spendable TxOuts.

mod account_ledger_scanner;

use crate::Error;
use account_ledger_scanner::AccountLedgerScanner;
use mc_account_keys::AccountKey;
use mc_blockchain_types::BlockIndex;
use mc_common::logger::Logger;
use mc_ledger_db::LedgerDB;
use mc_transaction_core::{ring_signature::KeyImage, tx::TxOut, Amount};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// An event emitted by the wallet.
#[derive(Clone, Debug)]
pub enum WalletEvent {
    /// A block has been processed.
    BlockProcessed {
        /// The index of the block that was processed.
        block_index: BlockIndex,

        /// Spendable TxOuts found in the block.
        received_tx_outs: Vec<MatchedTxOut>,

        /// TxOuts that were spent in the block.
        spent_tx_outs: Vec<MatchedTxOut>,
    },
}

/// Callback for notifying the caller of wallet events.
pub type WalletEventCallback = Arc<dyn Fn(WalletEvent) + Send + Sync>;

/// A TxOut that was matched by the wallet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MatchedTxOut {
    pub tx_out: TxOut,
    pub amount: Amount,
    pub subaddress_index: u64,
    pub key_image: KeyImage,
}

/// Internal state of the wallet.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct State {
    next_block_index: BlockIndex,
    matched_tx_outs: HashMap<KeyImage, MatchedTxOut>,
}
impl State {
    pub fn load(
        state_file: &PathBuf,
        default_first_block_index: BlockIndex,
    ) -> Result<Self, Error> {
        if state_file.exists() {
            let bytes = std::fs::read(&state_file)?;
            Ok(mc_util_serial::deserialize(&bytes)?)
        } else {
            Ok(Self {
                next_block_index: default_first_block_index,
                ..Default::default()
            })
        }
    }
    pub fn save(&self, path: &PathBuf) -> Result<(), Error> {
        let bytes = mc_util_serial::serialize(&self)?;
        std::fs::write(&path, bytes)?;
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
        default_first_block_index: BlockIndex,
        wallet_event_callback: WalletEventCallback,
        logger: Logger,
    ) -> Result<MiniWallet, Error> {
        let state_file = state_file.as_ref().to_path_buf();

        let state = Arc::new(Mutex::new(State::load(
            &state_file,
            default_first_block_index,
        )?));

        let account_ledger_scanner = AccountLedgerScanner::new(
            ledger_db,
            account_key,
            state.clone(),
            state_file,
            wallet_event_callback,
            logger,
        );

        Ok(MiniWallet {
            state,
            _account_ledger_scanner: account_ledger_scanner,
        })
    }

    pub fn next_block_index(&self) -> BlockIndex {
        self.state.lock().unwrap().next_block_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_blockchain_types::BlockVersion;
    use mc_common::logger::test_with_logger;
    use mc_ledger_db::test_utils::{create_ledger, initialize_ledger, INITIALIZE_LEDGER_AMOUNT};
    use mc_transaction_core::{constants::RING_SIZE, Token, TokenId};
    use mc_transaction_core_test_utils::Mob;
    use rand::{rngs::StdRng, SeedableRng};
    use std::{
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };
    use tempfile::tempdir;

    #[test_with_logger]
    fn mini_wallet_properly_identifies_spendable_and_spent_txos(logger: Logger) {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let account_key = AccountKey::random(&mut rng);

        let mut ledger_db = create_ledger();
        let n_blocks: usize = 3;
        initialize_ledger(
            BlockVersion::MAX,
            &mut ledger_db,
            n_blocks as u64,
            &account_key,
            &mut rng,
        );

        let events = Arc::new(Mutex::new(Vec::new()));
        let events2 = events.clone();

        let temp_dir = tempdir().unwrap();
        let _mini_wallet = MiniWallet::new(
            temp_dir.path().join("wallet-state"),
            ledger_db.clone(),
            account_key.clone(),
            0,
            Arc::new(move |event| events2.lock().unwrap().push(event)),
            logger.clone(),
        )
        .unwrap();

        // We expect to see 3 events, one for each block.
        for _ in 0..30 {
            if events.lock().unwrap().len() == n_blocks {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
        assert_eq!(events.lock().unwrap().len(), n_blocks);

        let mut expected_amount = Amount::new(INITIALIZE_LEDGER_AMOUNT, TokenId::MOB);

        for (i, event) in events.lock().unwrap().iter().enumerate() {
            match event {
                WalletEvent::BlockProcessed {
                    block_index,
                    received_tx_outs,
                    spent_tx_outs,
                } => {
                    // The first block has RING_SIZE spendable TxOuts, the rest have 1 (see
                    // initialize_ledger()).
                    let num_expected_tx_outs = if i == 0 { RING_SIZE } else { 1 };

                    // The first block does not spend anything, following blocks spend a single
                    // TxOut from the previous block. (See initialize_ledger())
                    let num_expected_spent_tx_outs = if i == 0 { 0 } else { 1 };

                    assert_eq!(*block_index, i as u64);
                    assert_eq!(received_tx_outs.len(), num_expected_tx_outs);
                    assert_eq!(spent_tx_outs.len(), num_expected_spent_tx_outs);

                    assert!(received_tx_outs
                        .iter()
                        .all(|matched_tx_out| matched_tx_out.amount == expected_amount));

                    if num_expected_spent_tx_outs > 0 {
                        assert!(spent_tx_outs
                            .iter()
                            .all(|matched_tx_out| matched_tx_out.amount.value
                                == expected_amount.value + Mob::MINIMUM_FEE));
                    }

                    // initialize_ledger spends the previous TxOut, minus the fee.
                    expected_amount.value -= Mob::MINIMUM_FEE;
                }
            }
        }
    }
}
