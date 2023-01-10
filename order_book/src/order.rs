// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, OrderId, Pair};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use std::ops::Deref;

/// A single "order" in the book. This is a wrapper around an SCI and some
/// auxiliary data
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Order {
    /// SCI
    sci: SignedContingentInput,

    /// Unique identifier
    id: OrderId,

    /// The pair being traded.
    pair: Pair,
}

impl Order {
    /// Get underlying SCI.
    pub fn sci(&self) -> &SignedContingentInput {
        &self.sci
    }

    /// Get unique identifier.
    pub fn id(&self) -> &OrderId {
        &self.id
    }

    /// Get the pair being traded by this order.
    pub fn pair(&self) -> &Pair {
        &self.pair
    }
}

impl TryFrom<SignedContingentInput> for Order {
    type Error = Error;

    fn try_from(sci: SignedContingentInput) -> Result<Self, Self::Error> {
        sci.validate()?;

        // TODO: We currently only support a single output, whether required or partial.
        // We do not support SCIs without input rules since that is handing out the
        // input for free.
        let input_rules = if let Some(input_rules) = sci.tx_in.input_rules.as_ref() {
            input_rules
        } else {
            return Err(Error::UnsupportedSci("Missing input rules".into()));
        };

        let counter_token_id = match (
            input_rules.required_outputs.len(),
            input_rules.partial_fill_outputs.len(),
        ) {
            (0, 0) => return Err(Error::UnsupportedSci("No required/partial outputs".into())),
            (1, 0) => {
                // Single required non-partial output
                TokenId::from(sci.required_output_amounts[0].token_id)
            }
            (0, 1) => {
                // Single partial output
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount().unwrap(); // TODO
                amount.token_id
            }
            _ => {
                return Err(Error::UnsupportedSci(format!(
                    "{}/{} required/partial outputs, expected 1/0 or 0/1",
                    input_rules.required_outputs.len(),
                    input_rules.partial_fill_outputs.len()
                )))
            }
        };

        let id = OrderId::from(&sci);

        let pair = Pair {
            base_token_id: TokenId::from(sci.pseudo_output_amount.token_id),
            counter_token_id,
        };

        Ok(Self { sci, id, pair })
    }
}

impl Deref for Order {
    type Target = SignedContingentInput;

    fn deref(&self) -> &Self::Target {
        &self.sci
    }
}
