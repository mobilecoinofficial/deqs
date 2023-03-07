// Copyright (c) 2023 MobileCoin Inc.

use crate::{
    mini_wallet::{MatchedTxOut, State, WalletEvent, WalletEventCallback},
    Error,
};
use mc_account_keys::{AccountKey, CHANGE_SUBADDRESS_INDEX, DEFAULT_SUBADDRESS_INDEX};
use mc_blockchain_types::{BlockContents, BlockIndex};
use mc_common::logger::{log, Logger};
use mc_crypto_keys::RistrettoPublic;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_transaction_core::{
    get_tx_out_shared_secret,
    onetime_keys::{recover_onetime_private_key, recover_public_subaddress_spend_key},
    ring_signature::KeyImage,
    tx::TxOut,
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::{
    cmp::min,
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// How many blocks to try and scan at each iteration.
const SCAN_CHUNK_SIZE: usize = 1000;

/// Time between periodic progress log messages.
const LOG_INTERVAL: Duration = Duration::from_secs(1);

/// Polling interval when waiting for new blocks.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Time to wait after retrying a failed operation.
const ERROR_SLEEP_INTERVAL: Duration = Duration::from_secs(1);

pub struct AccountLedgerScanner {
    stop_requested: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}
impl AccountLedgerScanner {
    pub fn new(
        ledger_db: LedgerDB,
        account_key: AccountKey,
        state: Arc<Mutex<State>>,
        state_file: PathBuf,
        wallet_event_callback: WalletEventCallback,
        logger: Logger,
    ) -> AccountLedgerScanner {
        // TODO - for now, we only support a few hardcoded subaddresses.
        let mut spsk_to_index = HashMap::new();
        for idx in [
            DEFAULT_SUBADDRESS_INDEX,
            1, // Historical change subaddress
            CHANGE_SUBADDRESS_INDEX,
        ]
        .iter()
        {
            spsk_to_index.insert(
                *account_key.subaddress(*idx).spend_public_key(),
                *idx as u64,
            );
        }

        let stop_requested = Arc::new(AtomicBool::new(false));
        let thread_stop_requested = stop_requested.clone();

        let join_handle = Some(
            thread::Builder::new()
                .name("AccountLedgerScanner".into())
                .spawn(move || {
                    let thread = AccountLedgerScannerWorker {
                        ledger_db,
                        account_key,
                        logger,
                        last_log_time: Instant::now(),
                        spsk_to_index,
                        state,
                        state_file,
                        wallet_event_callback,
                        stop_requested: thread_stop_requested,
                    };
                    thread.run();
                })
                .expect("Could not spawn thread"),
        );
        AccountLedgerScanner {
            join_handle,
            stop_requested,
        }
    }
}

impl Drop for AccountLedgerScanner {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::SeqCst);
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().expect("Could not join thread");
        }
    }
}

struct ScannedBlock {
    block_index: BlockIndex,
    matched_tx_outs: Vec<MatchedTxOut>,
    key_images: Vec<KeyImage>,
}

struct AccountLedgerScannerWorker {
    ledger_db: LedgerDB,
    account_key: AccountKey,
    logger: Logger,
    last_log_time: Instant,
    spsk_to_index: HashMap<RistrettoPublic, u64>,
    state: Arc<Mutex<State>>,
    state_file: PathBuf,
    wallet_event_callback: WalletEventCallback,
    stop_requested: Arc<AtomicBool>,
}
impl AccountLedgerScannerWorker {
    pub fn run(mut self) {
        while !self.stop_requested.load(Ordering::SeqCst) {
            let ledger_last_block_index = self
                .ledger_db
                .num_blocks()
                .expect("Could not get num blocks")
                - 1;
            let next_block_index = self.state.lock().expect("mutex poisoned").next_block_index;
            let last_block_index = min(
                ledger_last_block_index + 1,
                next_block_index + SCAN_CHUNK_SIZE as u64,
            );

            log::trace!(
                self.logger,
                "Trying to scan block {}-{}",
                next_block_index,
                last_block_index,
            );

            if self.last_log_time.elapsed() > LOG_INTERVAL
                && next_block_index < ledger_last_block_index
            {
                log::info!(
                    self.logger,
                    "Scanned up to block {} (ledger last block index is {})",
                    next_block_index,
                    ledger_last_block_index
                );
                self.last_log_time = Instant::now();
            }

            // Try to load and scan blocks in parallel.
            let results = match (next_block_index..last_block_index)
                .into_par_iter()
                .map(|block_index| {
                    let block_contents = self.ledger_db.get_block_contents(block_index);
                    match block_contents {
                        Ok(block_contents) => {
                            Ok(Some(self.process_block(block_index, block_contents)?))
                        }
                        Err(mc_ledger_db::Error::NotFound) => Ok(None),
                        Err(err) => Err(err.into()),
                    }
                })
                .collect::<Result<Vec<_>, Error>>()
            {
                Ok(results) => results,
                Err(err) => {
                    // While we retry here, the reality is tha the block scanning part really isn't
                    // expected to fail with the current LedgerDB/process_block
                    // implementation...
                    log::error!(self.logger, "Error scanning blocks: {:?}. Will retry.", err);
                    thread::sleep(ERROR_SLEEP_INTERVAL);
                    continue;
                }
            };

            // Process the results.
            let mut state = self.state.lock().expect("mutex poisoned");
            let orig_next_block_index = state.next_block_index;
            for result in results.into_iter().flatten() {
                assert_eq!(result.block_index, state.next_block_index);
                state.next_block_index += 1;

                let spent_tx_outs = result
                    .key_images
                    .iter()
                    // This implicitly skips key images that we identified as ours but failed to
                    // find a MatchedTxOut for. A more robust implementation
                    // would error when this happens, since it is unexpected unless we started
                    // scanning from the middle of the ledger.
                    .filter_map(|key_image| {
                        state
                            .matched_tx_outs
                            .remove(key_image)
                            .map(|matched_tx_out| {
                                log::info!(
                                    self.logger,
                                    "Recognized keyimage {} belonging to our account at block {}",
                                    matched_tx_out.key_image,
                                    result.block_index
                                );
                                matched_tx_out
                            })
                    })
                    .collect::<Vec<_>>();

                // Update our map of key image -> MatchedTxOut so that we can track spending.
                for matched_tx_out in result.matched_tx_outs.clone() {
                    state
                        .matched_tx_outs
                        .insert(matched_tx_out.key_image, matched_tx_out);
                }

                (self.wallet_event_callback)(WalletEvent::BlockProcessed {
                    block_index: result.block_index,
                    received_tx_outs: result.matched_tx_outs,
                    spent_tx_outs,
                });
            }
            state.save(&self.state_file).expect("Could not save state");
            let next_block_index = state.next_block_index;
            drop(state); // Release the lock before we sleep.

            if orig_next_block_index == next_block_index {
                // We didn't find any new blocks, so sleep for a bit.
                thread::sleep(POLL_INTERVAL);
            }
        }
    }

    fn process_block(
        &self,
        block_index: BlockIndex,
        block_contents: BlockContents,
    ) -> Result<ScannedBlock, Error> {
        let mut matched_tx_outs = Vec::new();

        for tx_out in block_contents.outputs {
            if let Some(matched_tx_out) = self.match_tx_out(&tx_out)? {
                log::info!(
                    self.logger,
                    "Found tx_out of amount {:?} at block {}",
                    matched_tx_out.amount,
                    block_index,
                );

                matched_tx_outs.push(matched_tx_out);
            }
        }

        Ok(ScannedBlock {
            block_index,
            matched_tx_outs,
            // We include all key images and not do the filtering in this function since we can only
            // filter after we placed all newly discovered TxOuts in our state map, and this only
            // happens after we are done parallel-scanning blocks.
            key_images: block_contents.key_images,
        })
    }

    fn match_tx_out(&self, tx_out: &TxOut) -> Result<Option<MatchedTxOut>, Error> {
        // This is view key scanning part, getting the value fails if view-key scanning
        // fails
        let decompressed_tx_pub = RistrettoPublic::try_from(&tx_out.public_key)?;
        let shared_secret =
            get_tx_out_shared_secret(self.account_key.view_private_key(), &decompressed_tx_pub);
        let (amount, _blinding) = match tx_out
            .get_masked_amount()
            .map_err(Error::from)
            .and_then(|masked_amount| masked_amount.get_value(&shared_secret).map_err(Error::from))
        {
            Ok((amount, blinding)) => (amount, blinding),
            Err(_) => return Ok(None),
        };

        // Calculate the subaddress spend public key for tx_out.
        let tx_out_target_key = RistrettoPublic::try_from(&tx_out.target_key)?;
        let tx_public_key = RistrettoPublic::try_from(&tx_out.public_key)?;

        let subaddress_spk = recover_public_subaddress_spend_key(
            self.account_key.view_private_key(),
            &tx_out_target_key,
            &tx_public_key,
        );
        let subaddress_index = match self.spsk_to_index.get(&subaddress_spk) {
            Some(index) => *index,
            None => {
                log::info!(
                    self.logger,
                    "Found tx_out pub key {} with amount {:?} with unknown subaddress index",
                    hex::encode(&tx_out.public_key),
                    amount,
                );
                return Ok(None);
            }
        };

        // This is the part where we compute the key image from the one-time private key
        let onetime_private_key = recover_onetime_private_key(
            &decompressed_tx_pub,
            self.account_key.view_private_key(),
            &self.account_key.subaddress_spend_private(subaddress_index),
        );
        let key_image = KeyImage::from(&onetime_private_key);

        // Done
        Ok(Some(MatchedTxOut {
            tx_out: tx_out.clone(),
            amount,
            subaddress_index,
            key_image,
        }))
    }
}
