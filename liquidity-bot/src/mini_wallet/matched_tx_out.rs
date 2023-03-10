// Copyright (c) 2023 MobileCoin Inc.

use crate::Error;
use mc_account_keys::AccountKey;
use mc_blockchain_types::BlockIndex;
use mc_common::logger::{log, Logger};
use mc_crypto_keys::RistrettoPublic;
use mc_transaction_core::{
    get_tx_out_shared_secret,
    onetime_keys::{recover_onetime_private_key, recover_public_subaddress_spend_key},
    ring_signature::KeyImage,
    tx::TxOut,
    Amount,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A TxOut that was tched by the wallet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MatchedTxOut {
    pub block_index: BlockIndex,
    pub tx_out: TxOut,
    pub amount: Amount,
    pub subaddress_index: u64,
    pub key_image: KeyImage,
}
impl MatchedTxOut {
    pub fn view_key_scan(
        block_index: BlockIndex,
        tx_out: &TxOut,
        account_key: &AccountKey,
        spsk_to_index: &HashMap<RistrettoPublic, u64>,
        logger: &Logger,
    ) -> Result<Option<Self>, Error> {
        // This is view key scanning part, getting the value fails if view-key scanning
        // fails
        let decompressed_tx_pub = RistrettoPublic::try_from(&tx_out.public_key)?;
        let shared_secret =
            get_tx_out_shared_secret(account_key.view_private_key(), &decompressed_tx_pub);
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
            account_key.view_private_key(),
            &tx_out_target_key,
            &tx_public_key,
        );
        let subaddress_index = match spsk_to_index.get(&subaddress_spk) {
            Some(index) => *index,
            None => {
                log::info!(
                    logger,
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
            account_key.view_private_key(),
            &account_key.subaddress_spend_private(subaddress_index),
        );
        let key_image = KeyImage::from(&onetime_private_key);

        log::trace!(
            logger,
            "Found tx_out pub key {} with amount {:?} with subaddress index {}",
            hex::encode(&tx_out.public_key),
            amount,
            subaddress_index,
        );

        // Done
        Ok(Some(Self {
            block_index,
            tx_out: tx_out.clone(),
            amount,
            subaddress_index,
            key_image,
        }))
    }
}
