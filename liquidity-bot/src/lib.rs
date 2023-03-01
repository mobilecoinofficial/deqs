// Copyright (c) 2023 MobileCoin Inc.

// TODO:
// - The bot loses all state when it dies, and the wallet will not re-feed it
//   TxOuts. Even if it did re-feed it, we wouldn't know which ones where
//   already previously submitted to the DEQS. As such, we will need to persist
//   the bot's state

mod config;
mod error;
pub mod mini_wallet;

pub use config::Config;
pub use error::Error;

use deqs_api::{
    deqs::{QuoteStatusCode, SubmitQuotesRequest, SubmitQuotesResponse},
    deqs_grpc::DeqsClientApiClient,
    DeqsClientUri,
};
use deqs_quote_book_api::QuoteId;
use grpcio::{ChannelBuilder, EnvBuilder};
use mc_common::logger::{log, Logger};
use mc_crypto_ring_signature_signer::{LocalRingSigner, OneTimeKeyDeriveData};
use mc_fog_report_resolver::FogResolver;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_transaction_builder::{
    EmptyMemoBuilder, InputCredentials, ReservedSubaddresses, SignedContingentInputBuilder,
};
use mc_transaction_core::{AccountKey, Amount, BlockVersion, TokenId};
use mc_transaction_extra::SignedContingentInput;
use mc_util_grpc::ConnectionUriGrpcioChannel;
use mini_wallet::{MatchedTxOut, WalletEvent};
use rand::Rng;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, time::interval};

/// A TxOut we want to submit to the DEQS
#[derive(Clone, Debug)]
pub struct PendingTxOut {
    /// The TxOut we are tracking
    matched_tx_out: MatchedTxOut,

    /// The SCI we generated from the TxOut
    sci: SignedContingentInput,
}

/// A TxOut we listed on the DEQS
#[derive(Clone, Debug)]
pub struct ListedTxOut {
    /// The TxOut we are tracking
    matched_tx_out: MatchedTxOut,

    /// The SCI we generated from the TxOut
    sci: SignedContingentInput,

    /// The quote we got from the DEQS (if we successfully submitted it).
    quote: deqs_api::deqs::Quote,

    /// Last time we tried to submit this TxOut to the DEQS
    last_submitted_at: Instant,
}

struct LiquidityBotTask {
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
    pending_tx_outs: Vec<PendingTxOut>,

    /// List of TxOuts we successfully submitted to the DEQS.
    listed_tx_outs: Vec<ListedTxOut>,

    /// Logger.
    logger: Logger,

    /// Wallet event receiver.
    wallet_event_rx: mpsc::UnboundedReceiver<WalletEvent>,

    /// DEQS client.
    deqs_client: DeqsClientApiClient,
}
impl LiquidityBotTask {
    pub async fn run(mut self) {
        // TODO
        let mut resubmit_tx_outs_interval = interval(Duration::from_secs(1));
        resubmit_tx_outs_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                event = self.wallet_event_rx.recv() => {
                    match event {
                        Some(WalletEvent::ReceivedTxOut { matched_tx_out }) => {
                            match self.add_tx_out(matched_tx_out).await {
                                Ok(_) => {
                                    if let Err(err) = self.submit_pending_tx_outs().await {
                                        log::info!(self.logger, "Error resubmitting TxOuts: {}", err);
                                    }

                                }
                                Err(err) => {
                                    log::error!(self.logger, "Error adding TxOut: {}", err);
                                }
                            }
                        }
                        Some(WalletEvent::SpentTxOut { .. }) => {
                            // TODO
                        }
                        None => {
                            log::error!(self.logger, "Wallet event receiver closed");
                            break;
                        }
                    }
                }

                _ = resubmit_tx_outs_interval.tick() => {
                    if let Err(err) = self.submit_pending_tx_outs().await {
                        log::info!(self.logger, "Error submitting pending TxOuts: {}", err);
                    }
                    if let Err(err) = self.resubmit_listed_tx_outs().await {
                        log::info!(self.logger, "Error resubmitting TxOuts: {}", err);
                    }
                }
            }
        }
    }

    async fn add_tx_out(&mut self, matched_tx_out: MatchedTxOut) -> Result<(), Error> {
        if self.pairs.contains_key(&matched_tx_out.amount.token_id) {
            let tracked_tx_out = self.create_pending_tx_out(matched_tx_out)?;
            self.pending_tx_outs.push(tracked_tx_out);
        }

        Ok(())
    }

    async fn submit_pending_tx_outs(&mut self) -> Result<(), Error> {
        if self.pending_tx_outs.is_empty() {
            return Ok(());
        }

        // TODO should batch in case we have too many
        let scis = self
            .pending_tx_outs
            .iter()
            .map(|tracked_tx_out| (&tracked_tx_out.sci).into())
            .collect();
        let mut req = SubmitQuotesRequest::default();
        req.set_quotes(scis);

        let resp = self.deqs_client.submit_quotes_async(&req)?.await?;
        sanity_check_submit_quotes_response(&resp, req.quotes.len())?;

        for (pending_tx_out, status_code, error_msg, quote) in itertools::izip!(
            self.pending_tx_outs.drain(..),
            &resp.status_codes,
            &resp.error_messages,
            &resp.quotes
        ) {
            match status_code {
                QuoteStatusCode::CREATED | QuoteStatusCode::QUOTE_ALREADY_EXISTS => {
                    let quote_id = QuoteId::try_from(quote.get_id())?;

                    // Check if the SCI is actually the one we submitted, or a different one.
                    let quote_sci = SignedContingentInput::try_from(quote.get_sci())?;
                    if quote_sci == pending_tx_out.sci {
                        log::info!(
                            self.logger,
                            "Submitted TxOut {} to DEQS, quote id is {}",
                            hex::encode(pending_tx_out.matched_tx_out.tx_out.public_key),
                            quote_id
                        );
                    } else {
                        assert_eq!(status_code, &QuoteStatusCode::QUOTE_ALREADY_EXISTS);
                        log::warn!(
                            self.logger,
                            "DEQS returned a different SCI than the one we submitted for TxOut {}",
                            hex::encode(pending_tx_out.matched_tx_out.tx_out.public_key),
                        );
                    }

                    // NOTE: We are taking the SCI that is returned from the DEQS, and not the one
                    // we submitted. We do this because it is possible that the
                    // DEQS will already have an SCI for the TxOut we just tried
                    // using. For example, this could happen when the bot restarts.
                    // The important implication of this is that once we pick up whatever SCI we got
                    // from the DEQS and put it in our `listed_tx_outs` list, we
                    // will keep re-submitting it (trying to keep it listed). In
                    // the future we might want to change this behavior to only do this if the swap
                    // rate is the one we are configured for. Some other options
                    // for followup work are:
                    //
                    // 1. Have the bot persist which SCIs it submitted, so after a restart it does
                    // not get surprised when its  TxOutss are already listed.
                    //
                    // 2. Have the bot cancel SCIs that are not the one it submitted, but this would
                    // only make sense if it persists state since otherwise it will cancel all of
                    // its SCIs every time it restarts.
                    let listed_tx_out = ListedTxOut {
                        matched_tx_out: pending_tx_out.matched_tx_out,
                        sci: quote_sci,
                        quote: quote.clone(),
                        last_submitted_at: Instant::now(),
                    };
                    self.listed_tx_outs.push(listed_tx_out);
                }

                QuoteStatusCode::QUOTE_IS_STALE => {
                    log::info!(
                        self.logger,
                        "DEQS rejected TxOut {}: quote is stale",
                        hex::encode(pending_tx_out.matched_tx_out.tx_out.public_key)
                    );
                }

                err => {
                    log::error!(
                        self.logger,
                        "DEQS rejected TxOut {} for a reason we did not expect: {:?} ({})",
                        hex::encode(pending_tx_out.matched_tx_out.tx_out.public_key),
                        err,
                        error_msg,
                    );
                }
            }
        }

        Ok(())
    }

    async fn resubmit_listed_tx_outs(&mut self) -> Result<(), Error> {
        // Split our listed list into two: those that we want to try and resubmit and
        // those that have been resubmitted recently enough.
        let (to_resubmit, to_keep) =
            self.listed_tx_outs
                .drain(..)
                .partition::<Vec<_>, _>(|listed_tx_out| {
                    listed_tx_out.last_submitted_at.elapsed() > Duration::from_secs(10)
                    // TODO make resubmit time configurable
                });
        self.listed_tx_outs = to_keep;

        if to_resubmit.is_empty() {
            return Ok(());
        }

        log::debug!(
            self.logger,
            "Need to resubmit {} TxOuts and hold on {} TxOuts",
            to_resubmit.len(),
            self.listed_tx_outs.len()
        );

        // TODO should batch in case we have too many to resubmit

        let scis = to_resubmit
            .iter()
            .map(|tracked_tx_out| (&tracked_tx_out.sci).into())
            .collect();
        let mut req = SubmitQuotesRequest::default();
        req.set_quotes(scis);

        // Try and submit the quotes to the DEQS, if it fails we need to put them back
        // and assume they are still listed. We will try again later.
        let resp = match self.deqs_client.submit_quotes_async(&req) {
            Ok(async_resp) => match async_resp.await {
                Ok(resp) => resp,
                Err(err) => {
                    self.listed_tx_outs.extend(to_resubmit);
                    return Err(err.into());
                }
            },
            Err(err) => {
                self.listed_tx_outs.extend(to_resubmit);
                return Err(err.into());
            }
        };
        sanity_check_submit_quotes_response(&resp, req.quotes.len())?;

        for (mut listed_tx_out, status_code, error_msg, quote) in itertools::izip!(
            to_resubmit,
            &resp.status_codes,
            &resp.error_messages,
            &resp.quotes
        ) {
            let quote_id = QuoteId::try_from(quote.get_id())?;
            let quote_sci = SignedContingentInput::try_from(quote.get_sci())?;

            match status_code {
                QuoteStatusCode::CREATED => {
                    // Sanity tha the DEQS is behaving as expected.
                    assert_eq!(quote_sci, listed_tx_out.sci);

                    log::info!(
                        self.logger,
                        "Re-submitted TxOut {} to DEQS, quote id is {}",
                        hex::encode(listed_tx_out.matched_tx_out.tx_out.public_key),
                        quote_id
                    );
                    listed_tx_out.quote = quote.clone();
                    listed_tx_out.last_submitted_at = Instant::now();
                    self.listed_tx_outs.push(listed_tx_out);
                }

                QuoteStatusCode::QUOTE_ALREADY_EXISTS => {
                    if quote_sci == listed_tx_out.sci {
                        log::debug!(
                            self.logger,
                            "DEQS confirmed quote {} is still listed",
                            quote_id
                        );
                    } else {
                        log::warn!(
                            self.logger,
                            "DEQS returned a different SCI than the one we submitted for TxOut {}",
                            hex::encode(listed_tx_out.matched_tx_out.tx_out.public_key),
                        );

                        // See long comment in `submit_pending_tx_outs` about why we do this.
                        listed_tx_out.sci = quote_sci;
                    }

                    listed_tx_out.last_submitted_at = Instant::now();
                    self.listed_tx_outs.push(listed_tx_out);
                }

                QuoteStatusCode::QUOTE_IS_STALE => {
                    log::info!(self.logger, "Quote {} expired", quote_id);
                }

                err => {
                    // NOTE: Right now this implies we stop tracking the listed TxOut.
                    log::error!(
                        self.logger,
                        "DEQS rejected TxOut {} for a reason we did not expect: {:?} ({})",
                        hex::encode(listed_tx_out.matched_tx_out.tx_out.public_key),
                        err,
                        error_msg,
                    );
                }
            }
        }

        Ok(())
    }

    fn create_pending_tx_out(&self, matched_tx_out: MatchedTxOut) -> Result<PendingTxOut, Error> {
        let (counter_token_id, swap_rate) = self
            .pairs
            .get(&matched_tx_out.amount.token_id)
            .ok_or(Error::UnknownTokenId(matched_tx_out.amount.token_id))?;

        let counter_amount: u64 = (Decimal::from(matched_tx_out.amount.value) * swap_rate)
            .to_u64()
            .ok_or_else(|| {
                Error::DecimalConversion(format!(
                    "Could not convert {} times {} to u64",
                    matched_tx_out.amount.value, swap_rate
                ))
            })?;

        log::info!(
            self.logger,
            "Creating SCI for TxOut with amount {}: wanting {} counter tokens (token id {})",
            matched_tx_out.amount.value,
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
            .get_tx_out_index_by_public_key(&matched_tx_out.tx_out.public_key)?;
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
        let input_credentials = InputCredentials::new(
            tx_outs,
            proofs,
            0,
            OneTimeKeyDeriveData::SubaddressIndex(matched_tx_out.subaddress_index),
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
            matched_tx_out.amount,
            &ReservedSubaddresses::from(&self.account_key),
            &mut rng,
        )?;
        let sci = builder.build(&LocalRingSigner::from(&self.account_key), &mut rng)?;

        Ok(PendingTxOut {
            matched_tx_out,
            sci,
        })
    }
}

pub struct LiquidityBot {
    /// Wallet event tx
    wallet_event_tx: mpsc::UnboundedSender<WalletEvent>,
}
impl LiquidityBot {
    /// Construct a new LiquidityBot instance.
    pub fn new(
        account_key: AccountKey,
        ledger_db: LedgerDB,
        pairs: HashMap<TokenId, (TokenId, Decimal)>,
        deqs_uri: &DeqsClientUri,
        logger: Logger,
    ) -> Self {
        let (wallet_event_tx, wallet_event_rx) = mpsc::unbounded_channel();

        let env = Arc::new(EnvBuilder::new().name_prefix("deqs-client-grpc").build());
        let ch = ChannelBuilder::default_channel_builder(env).connect_to_uri(deqs_uri, &logger);
        let deqs_client = DeqsClientApiClient::new(ch);

        let task = LiquidityBotTask {
            account_key,
            ledger_db,
            pairs,
            pending_tx_outs: Vec::new(),
            listed_tx_outs: Vec::new(),
            wallet_event_rx,
            deqs_client,
            logger: logger.clone(),
        };
        tokio::spawn(task.run());
        Self { wallet_event_tx }
    }

    /// Bridge in a wallet event.
    pub fn notify_wallet_event(&self, event: WalletEvent) {
        self.wallet_event_tx
            .send(event)
            .expect("wallet event tx failed"); // We assume the channel stays
                                               // open for as long as the bot is
                                               // running.
    }
}

fn sanity_check_submit_quotes_response(
    resp: &SubmitQuotesResponse,
    expected_len: usize,
) -> Result<(), Error> {
    if resp.status_codes.len() != expected_len {
        return Err(Error::InvalidGrpcResponse(format!("Number of status codes in response does not match number of quotes in request. Expected {}, got {}.", expected_len, resp.status_codes.len())));
    }

    if resp.error_messages.len() != expected_len {
        return Err(Error::InvalidGrpcResponse(format!("Number of error messages in response does not match number of quotes in request. Expected {}, got {}.", expected_len, resp.error_messages.len())));
    }

    if resp.quotes.len() != expected_len {
        return Err(Error::InvalidGrpcResponse(format!("Number of quotes in response does not match number of quotes in request. Expected {}, got {}.", expected_len, resp.quotes.len())));
    }

    Ok(())
}
