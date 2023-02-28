// Copyright (c) 2023 MobileCoin Inc.

mod config;
mod account_ledger_scanner;

pub use config::Config;
pub use account_ledger_scanner::AccountLedgerScanner;

use displaydoc::Display;
use mc_common::logger::{log, Logger};
use mc_crypto_keys::{RistrettoPublic, KeyError};
use mc_crypto_ring_signature_signer::LocalRingSigner;
use mc_fog_report_resolver::FogResolver;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_transaction_builder::{
    EmptyMemoBuilder, InputCredentials, ReservedSubaddresses, SignedContingentInputBuilder,
    SignedContingentInputBuilderError, TxBuilderError,
};
use mc_transaction_core::{
    get_tx_out_shared_secret,
    onetime_keys::recover_onetime_private_key,
    tx::{TxOut, TxOutMembershipProof},
    AccountKey, Amount, BlockVersion, TokenId, TxOutConversionError, AmountError,
};
use mc_transaction_extra::SignedContingentInput;
use rand::Rng;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use std::collections::{HashMap, HashSet};

/// A TxOut

/// Possible statuses for a tracked TxOut
#[derive(Clone, Debug, Display)]
pub enum TrackedTxOutStatus {
    /// Pending submission to the DEQS
    PendingSubmission,

    /// Live - listed on the DEQS
    Live,

    /**
     * Canceled - the TxOut was spent but we didn't get the counter tokens
     * in return.
     */
    Canceled,

    /// Used - the TxOut was consumed and we got some counter tokens in return.
    Used,
}

/// A TxOut we keep track of
#[derive(Clone, Debug)]
pub struct TrackedTxOut {
    /// The TxOut we are tracking
    tx_out: TxOut,

    /// The SCI we generated from the TxOut
    sci: SignedContingentInput,

    /// The quote we got from the DEQS (if we successfully submitted it).
    quote: Option<deqs_api::deqs::Quote>,
}

pub struct LiquidityBot {
    /// Account key
    account_key: AccountKey,

    /// Ledger DB
    ledger_db: LedgerDB,

    /// Pairs we are interested in listing.
    /// This is a map of the base token to the counter token and the swap rate
    /// we are offering. The rate specifies how many counter tokens are
    /// needed to get one base token.
    pairs: HashMap<TokenId, (TokenId, Decimal)>,

    /// List of TxOuts we would like to submit to the DEQS.
    pending_tx_outs: Vec<TxOut>,

    /// Logger.
    logger: Logger,
}
impl LiquidityBot {
    fn create_tracked_tx_out(&self, tx_out: TxOut) -> Result<TrackedTxOut, Error> {
        let public_key = RistrettoPublic::try_from(&tx_out.public_key).unwrap(); // TODO

        let shared_secret =
            get_tx_out_shared_secret(self.account_key.view_private_key(), &public_key);

        let (amount, _blinding_factor) = tx_out
            .get_masked_amount()
            .expect("TxOut missing masked value") // TODO
            .get_value(&shared_secret)
            .expect("TxOut not owned by us"); // TODO

        let (counter_token_id, swap_rate) = self.pairs.get(&amount.token_id).unwrap(); // TODO

        let counter_amount: u64 = (Decimal::from(amount.value) * swap_rate).to_u64().unwrap(); // TODO
        log::debug!(
            self.logger,
            "Creating SCI for TxOut with amount {}: wanting {} counter tokens (token id {})",
            amount.value,
            counter_amount,
            counter_token_id,
        );

        // Construct a ring. The first step is to choose tx out indices.
        let mut rng = rand::thread_rng();
        let mut sampled_indices = HashSet::new();

        const RING_SIZE: usize = 11;
        let num_txos = self.ledger_db.num_txos()?;
        while sampled_indices.len() < RING_SIZE {
            let index = rng.gen_range(0..num_txos);
            sampled_indices.insert(index);
        }
        let mut sampled_indices_vec: Vec<u64> = sampled_indices.into_iter().collect();

        // Ensure our TxOut is in the ring. Always using index 0 is safe since
        // InputCredentials::new sorts the ring.
        let our_tx_out_index = self
            .ledger_db
            .get_tx_out_index_by_public_key(&tx_out.public_key)?;
        sampled_indices_vec[0] = our_tx_out_index;

        // Get the actual TxOuts
        let tx_outs = sampled_indices_vec
            .iter()
            .map(|idx| self.ledger_db.get_tx_out_by_index(*idx))
            .collect::<Result<Vec<_>, _>>()?;

        // Get proofs for all those indexes.
        let proofs = self
            .ledger_db
            .get_tx_out_proof_of_memberships(&sampled_indices_vec)?;

        // Create our InputCredentials
        let onetime_private_key = recover_onetime_private_key(
            &public_key,
            self.account_key.view_private_key(),
            &self.account_key.default_subaddress_spend_private(),
        );

        let input_credentials = InputCredentials::new(
            tx_outs,
            proofs,
            0,
            onetime_private_key, // TODO we dont need this if we use LocalRingSigner
            *self.account_key.view_private_key(),
        )?;

        // Build the SCI
        let block_version = BlockVersion::MAX;
        let fog_resolver = FogResolver::default(); // TODO

        let mut builder = SignedContingentInputBuilder::new(
            block_version,
            input_credentials,
            fog_resolver,
            EmptyMemoBuilder::default(), // TODO
        )?;
        builder.add_partial_fill_output(
            Amount::new(counter_amount, *counter_token_id),
            &self.account_key.default_subaddress(),
            &mut rng,
        )?;
        builder.add_partial_fill_change_output(
            amount,
            &ReservedSubaddresses::from(&self.account_key),
            &mut rng,
        )?;
        let sci = builder.build(&LocalRingSigner::from(&self.account_key), &mut rng)?;

        Ok(TrackedTxOut {
            tx_out,
            sci,
            quote: None,
        })
    }
}

use mc_ledger_db::Error as LedgerDbError;
#[derive(Debug, Display)]
pub enum Error {
    /// Ledger Db: {0}
    LedgerDb(LedgerDbError),

    /// Tx Builder: {0}
    TxBuilderError(TxBuilderError),

    /// Signed Contingent Input Builder: {0}
    SignedContingentInputBuilder(SignedContingentInputBuilderError),

    /// Amount: {0}
    Amount(AmountError),

    /// TxOut conversion: {0}
    TxOutConversion(TxOutConversionError),

    /// Crypto key: {0}
    Key(KeyError),
}
impl From<LedgerDbError> for Error {
    fn from(src: LedgerDbError) -> Self {
        Self::LedgerDb(src)
    }
}
impl From<TxBuilderError> for Error {
    fn from(src: TxBuilderError) -> Self {
        Self::TxBuilderError(src)
    }
}
impl From<SignedContingentInputBuilderError> for Error {
    fn from(src: SignedContingentInputBuilderError) -> Self {
        Self::SignedContingentInputBuilder(src)
    }
}
impl From<AmountError> for Error {
    fn from(src: AmountError) -> Self {
        Self::Amount(src)
    }
}
impl From<TxOutConversionError> for Error {
    fn from(src: TxOutConversionError) -> Self {
        Self::TxOutConversion(src)
    }
}
impl From<KeyError> for Error {
    fn from(src: KeyError) -> Self {
        Self::Key(src)
    }
}