// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, OrderId, Pair};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use std::ops::{Deref, RangeInclusive};

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

    /// The the range of base tokens offered by this order (the minimum and
    /// maximum amount of base token that can be obtained by fulfiling the
    /// order)
    base_range: RangeInclusive<u64>,
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

    /// Get the range of base tokens offered by this order (the minimum and
    /// maximum amount of base token that can be obtained by fulfiling the
    /// order).
    pub fn base_range(&self) -> &RangeInclusive<u64> {
        &self.base_range
    }

    // Get the number of counter tokens we will need to provide in order to consume
    // this SCI and receive a total of base_tokens back.
    pub fn counter_tokens_cost(&self, base_tokens: u64) -> Result<u64, Error> {
        if !self.base_range.contains(&base_tokens) {
            return Err(Error::CannotFulfilBaseTokens(base_tokens));
        }

        // TODO: This is making strong assumptions about the structure of the SCI and
        // doesn't currently take into account the scenario where we would also
        // want a fee output to pay the DEQS.
        let input_rules = if let Some(input_rules) = self.sci.tx_in.input_rules.as_ref() {
            input_rules
        } else {
            return Err(Error::UnsupportedSci("Missing input rules".into()));
        };

        match (
            input_rules.required_outputs.len(),
            input_rules.partial_fill_outputs.len(),
        ) {
            (0, 0) => return Err(Error::UnsupportedSci("No required/partial outputs".into())),
            (1, 0) => {
                // Single required non-partial output. This order can only execute if are taking
                // the entire amount.
                // The assert here makes sense since we should only get here if base_tokens is a
                // range containing only self.sci.pseudo_output_amount.value
                assert!(base_tokens == self.sci.pseudo_output_amount.value);
                Ok(self.sci.required_output_amounts[0].value)
            }
            (0, 1) => {
                // Single partial output. The amount we are taking must be above the partial
                // fill value, and below the max available amount.
                // The asserts here makes sense since we should only get here if base_range
                // (checked at the beginning of the function) is between
                // input_rules.min_partial_fill_value and self.sci.
                // pseudo_output_amount.value
                assert!(base_tokens >= input_rules.min_partial_fill_value);
                assert!(base_tokens <= self.sci.pseudo_output_amount.value);

                // The ratio being filled
                let fill_fraction_num: u128 = base_tokens as u128;
                let fill_fractions_denom = self.sci.pseudo_output_amount.value as u128;

                // Calculate the number of counter tokens we need to return as change to the
                // offerer of the SCI
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount().unwrap(); // TODO
                let num_128 = amount.value as u128 * fill_fraction_num;
                // Divide and round down
                Ok((num_128 / fill_fractions_denom) as u64)
            }
            (1, 1) => {
                // Single partial output with a required change output. The amount we are taking
                // must be above the partial fill value, and below the max
                // available amount.
                // The asserts here makes sense since we should only get here if base_range
                // (checked at the beginning of the function) is between
                // input_rules.min_partial_fill_value and max_available_amount
                assert!(
                    self.sci.pseudo_output_amount.value > self.sci.required_output_amounts[0].value
                );
                let max_available_amount =
                    self.sci.pseudo_output_amount.value - self.sci.required_output_amounts[0].value;
                assert!(base_tokens >= input_rules.min_partial_fill_value);
                assert!(base_tokens <= max_available_amount);

                // The ratio being filled
                let fill_fraction_num: u128 = base_tokens as u128;
                let fill_fractions_denom = self.sci.pseudo_output_amount.value as u128;

                // Calculate the number of counter tokens we need to return as change to the
                // offerer of the SCI
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount().unwrap(); // TODO
                let num_128 = amount.value as u128 * fill_fraction_num;
                // Divide and round down
                Ok((num_128 / fill_fractions_denom) as u64)
            }

            _ => {
                return Err(Error::UnsupportedSci(format!(
                    "Unsupported number of required/partial outputs {}/{}",
                    input_rules.required_outputs.len(),
                    input_rules.partial_fill_outputs.len()
                )))
            }
        }
    }
}

impl TryFrom<SignedContingentInput> for Order {
    type Error = Error;

    fn try_from(sci: SignedContingentInput) -> Result<Self, Self::Error> {
        sci.validate()?;

        // The base token being offered in exchange for some other token that the
        // fulfiller will provide.
        let base_token_id = TokenId::from(sci.pseudo_output_amount.token_id);

        // TODO: This is making strong assumptions about the structure of the SCI and
        // doesn't currently take into account the scenario where we would also
        // want a fee output to pay the DEQS.
        let input_rules = if let Some(input_rules) = sci.tx_in.input_rules.as_ref() {
            input_rules
        } else {
            return Err(Error::UnsupportedSci("Missing input rules".into()));
        };

        let (counter_token_id, base_range) = match (
            input_rules.required_outputs.len(),
            input_rules.partial_fill_outputs.len(),
        ) {
            (0, 0) => return Err(Error::UnsupportedSci("No required/partial outputs".into())),
            (1, 0) => {
                // Single required non-partial output
                (
                    TokenId::from(sci.required_output_amounts[0].token_id),
                    sci.pseudo_output_amount.value..=sci.pseudo_output_amount.value,
                )
            }
            (0, 1) => {
                // Single partial output
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount()?;
                let min_base_amount = input_rules.min_partial_fill_value;
                let max_base_amount = sci.pseudo_output_amount.value;

                (amount.token_id, min_base_amount..=max_base_amount)
            }
            (1, 1) => {
                // Partial output and what we think is a mandatory change output.
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount()?;
                let min_base_amount = input_rules.min_partial_fill_value;
                let max_base_amount = sci.pseudo_output_amount.value;

                // Verify that the required non-partial output appear to be a mandatory change
                // output
                if sci.required_output_amounts[0].token_id != base_token_id {
                    return Err(Error::UnsupportedSci(format!(
                        "Suspected required-change-output token id {} does not match partial output token id {}",
                        sci.required_output_amounts[0].token_id, amount.token_id
                    )));
                }

                // Max available amount needs to take the required change output into account.
                let max_available_amount = max_base_amount
                    .checked_sub(sci.required_output_amounts[0].value)
                    .ok_or_else(|| {
                        Error::UnsupportedSci(format!(
                            "max base amount {} is lower than required change {}",
                            max_base_amount, sci.required_output_amounts[0].value
                        ))
                    })?;

                (amount.token_id, min_base_amount..=max_available_amount)
            }
            _ => {
                return Err(Error::UnsupportedSci(format!(
                    "Unsupported number of required/partial outputs {}/{}",
                    input_rules.required_outputs.len(),
                    input_rules.partial_fill_outputs.len()
                )))
            }
        };

        let id = OrderId::from(&sci);

        let pair = Pair {
            base_token_id,
            counter_token_id,
        };

        Ok(Self {
            sci,
            id,
            pair,
            base_range,
        })
    }
}

impl Deref for Order {
    type Target = SignedContingentInput;

    fn deref(&self) -> &Self::Target {
        &self.sci
    }
}

#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ directory since we want to be able to
    // re-use some test code between implementations and that seems to be the
    // way to make Rust do that.
}
