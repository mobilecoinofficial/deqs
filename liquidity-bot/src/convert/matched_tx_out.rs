// Copyright (c) 2023 MobileCoin Inc.

use crate::mini_wallet::MatchedTxOut;
use deqs_liquidity_bot_api::{liquidity_bot as api, ConversionError};

impl From<&MatchedTxOut> for api::MatchedTxOut {
    fn from(src: &MatchedTxOut) -> Self {
        let mut dst = api::MatchedTxOut::new();
        dst.set_block_index(src.block_index);
        dst.set_tx_out((&src.tx_out).into());
        dst.set_amount((&src.amount).into());
        dst.set_subaddress_index(src.subaddress_index);
        dst.set_key_image((&src.key_image).into());
        dst
    }
}

impl TryFrom<&api::MatchedTxOut> for MatchedTxOut {
    type Error = ConversionError;

    fn try_from(src: &api::MatchedTxOut) -> Result<Self, Self::Error> {
        Ok(Self {
            block_index: src.get_block_index(),
            tx_out: src.get_tx_out().try_into()?,
            amount: src.get_amount().try_into()?,
            subaddress_index: src.get_subaddress_index(),
            key_image: src.get_key_image().try_into()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_account_keys::PublicAddress;
    use mc_blockchain_types::BlockVersion;
    use mc_crypto_keys::RistrettoPrivate;
    use mc_transaction_core::Amount;
    use mc_transaction_core_test_utils::{KeyImage, TxOut};
    use mc_util_from_random::{FromRandom, RngCore};
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn roundtrip() {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

        let amount = Amount {
            value: 1u64 << 13,
            token_id: 123.into(),
        };
        let tx_out = TxOut::new(
            BlockVersion::MAX,
            amount,
            &PublicAddress::from_random(&mut rng),
            &RistrettoPrivate::from_random(&mut rng),
            Default::default(),
        )
        .unwrap();

        let src = MatchedTxOut {
            block_index: 123,
            tx_out,
            amount,
            subaddress_index: 456,
            key_image: KeyImage::from(rng.next_u64()),
        };

        let converted = api::MatchedTxOut::from(&src);
        let recovered = MatchedTxOut::try_from(&converted).unwrap();
        assert_eq!(src, recovered);
    }
}
