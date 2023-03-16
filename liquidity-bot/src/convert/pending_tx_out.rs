// Copyright (c) 2023 MobileCoin Inc.

use crate::PendingTxOut;
use deqs_liquidity_bot_api::{liquidity_bot as api, ConversionError};

impl From<&PendingTxOut> for api::PendingTxOut {
    fn from(src: &PendingTxOut) -> Self {
        let mut dst = api::PendingTxOut::new();
        dst.set_matched_tx_out((&src.matched_tx_out).into());
        dst.set_sci((&src.sci).into());
        dst
    }
}

impl TryFrom<&api::PendingTxOut> for PendingTxOut {
    type Error = ConversionError;

    fn try_from(src: &api::PendingTxOut) -> Result<Self, Self::Error> {
        Ok(Self {
            matched_tx_out: src.get_matched_tx_out().try_into()?,
            sci: src.get_sci().try_into()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_account_keys::{AccountKey, PublicAddress};
    use mc_blockchain_types::BlockVersion;
    use mc_crypto_keys::RistrettoPrivate;
    use mc_crypto_ring_signature_signer::NoKeysRingSigner;
    use mc_transaction_builder::{
        test_utils::get_input_credentials, EmptyMemoBuilder, SignedContingentInputBuilder,
    };
    use mc_transaction_core::{constants::MILLIMOB_TO_PICOMOB, Amount, Token, TokenId};
    use mc_transaction_core_test_utils::{KeyImage, Mob, MockFogResolver, TxOut};
    use mc_util_from_random::{FromRandom, RngCore};
    use rand::{rngs::StdRng, SeedableRng};

    use crate::{mini_wallet::MatchedTxOut, PendingTxOut};

    #[test]
    fn roundtrip() {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let charlie = AccountKey::random(&mut rng);
        let token2 = TokenId::from(2);
        let fpr = MockFogResolver::default();
        let input_credentials = get_input_credentials(
            BlockVersion::MAX,
            Amount::new(1000, token2),
            &charlie,
            &fpr,
            &mut rng,
        );
        let mut sci_builder = SignedContingentInputBuilder::new(
            BlockVersion::MAX,
            input_credentials,
            fpr.clone(),
            EmptyMemoBuilder::default(),
        )
        .unwrap();
        sci_builder
            .add_partial_fill_output(
                Amount::new(1000 * MILLIMOB_TO_PICOMOB, Mob::ID),
                &charlie.default_subaddress(),
                &mut rng,
            )
            .unwrap();

        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

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
        let matched_tx_out = MatchedTxOut {
            block_index: 123,
            tx_out,
            amount,
            subaddress_index: 456,
            key_image: KeyImage::from(rng.next_u64()),
        };

        let src = PendingTxOut {
            matched_tx_out,
            sci,
        };
        let converted = api::PendingTxOut::from(&src);
        let recovererd = PendingTxOut::try_from(&converted).unwrap();
        assert_eq!(src, recovererd);
    }
}
