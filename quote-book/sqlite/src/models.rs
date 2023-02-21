// Copyright (c) 2023 MobileCoin Inc.

use crate::{schema, sql_types::VecU64};
use deqs_quote_book_api::{Error, Pair, QuoteId};
use diesel::{Insertable, Queryable};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use mc_util_serial::{decode, encode};
use std::ops::RangeInclusive;

#[derive(Queryable, Insertable)]
#[diesel(table_name = schema::quotes)]
pub struct Quote {
    id: Vec<u8>,
    sci_protobuf: Vec<u8>,
    base_token_id: i64,
    counter_token_id: i64,
    base_range_min: VecU64,
    base_range_max: VecU64,
    max_counter_tokens: VecU64,
    timestamp: i64,
    key_image: Vec<u8>,
    tombstone_block: i64,
}

impl Quote {
    pub fn id(&self) -> Result<QuoteId, Error> {
        if self.id.len() != 32 {
            return Err(Error::ImplementationSpecific(format!(
                "Expected 32 bytes quote id, found {} bytes instead",
                self.id.len(),
            )));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&self.id);
        Ok(QuoteId(id))
    }
    pub fn decode_sci(&self) -> Result<SignedContingentInput, Error> {
        decode(&self.sci_protobuf).map_err(|e| Error::ImplementationSpecific(e.to_string()))
    }
    pub fn base_token_id(&self) -> TokenId {
        TokenId::from(self.base_token_id as u64)
    }
    pub fn counter_token_id(&self) -> TokenId {
        TokenId::from(self.counter_token_id as u64)
    }
    pub fn pair(&self) -> Pair {
        Pair {
            base_token_id: self.base_token_id(),
            counter_token_id: self.counter_token_id(),
        }
    }
    pub fn base_range(&self) -> RangeInclusive<u64> {
        let min = *self.base_range_min;
        let max = *self.base_range_max;
        min..=max
    }
    pub fn max_counter_tokens(&self) -> u64 {
        *self.max_counter_tokens
    }
    pub fn timestamp(&self) -> u64 {
        self.timestamp as u64
    }
}

impl From<&deqs_quote_book_api::Quote> for Quote {
    fn from(quote: &deqs_quote_book_api::Quote) -> Self {
        let id = quote.id().0.to_vec();
        let sci_protobuf = encode(quote.sci());
        let base_token_id = *quote.pair().base_token_id as i64;
        let counter_token_id = *quote.pair().counter_token_id as i64;
        let base_range_min = quote.base_range().start().into();
        let base_range_max = quote.base_range().end().into();
        let max_counter_tokens = quote.max_counter_tokens().into();
        let timestamp = quote.timestamp() as i64;
        let key_image = quote.sci().key_image().to_vec();
        let tombstone_block = quote
            .sci()
            .tx_in
            .input_rules
            .as_ref()
            .map(|input_rules| input_rules.max_tombstone_block)
            .unwrap_or(0) as i64; // 0 implies no max tombstone block
        Self {
            id,
            sci_protobuf,
            base_token_id,
            counter_token_id,
            base_range_min,
            base_range_max,
            max_counter_tokens,
            timestamp,
            key_image,
            tombstone_block,
        }
    }
}

impl TryFrom<&Quote> for deqs_quote_book_api::Quote {
    type Error = Error;

    fn try_from(quote: &Quote) -> Result<Self, Self::Error> {
        let id = quote.id()?;
        let sci = quote.decode_sci()?;
        let pair = quote.pair();
        let base_range = quote.base_range();
        let max_counter_tokens = quote.max_counter_tokens();
        let timestamp = quote.timestamp();
        Ok(Self::new_from_fields(
            sci,
            id,
            pair,
            base_range,
            max_counter_tokens,
            timestamp,
        ))
    }
}
