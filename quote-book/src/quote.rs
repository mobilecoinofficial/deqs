// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, Pair, QuoteId};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use std::{
    cmp::Ordering,
    ops::{Deref, RangeInclusive},
    time::{SystemTime, UNIX_EPOCH},
};

/// A single "quote" in the book. This is a wrapper around an SCI and some
/// auxiliary data
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Quote {
    /// SCI
    sci: SignedContingentInput,

    /// Unique identifier
    id: QuoteId,

    /// The pair being traded.
    pair: Pair,

    /// The the range of base tokens offered by this quote (the minimum and
    /// maximum amount of base token that can be obtained by fulfiling the
    /// quote)
    base_range: RangeInclusive<u64>,

    /// The number of counter tokens needed to trade the max amount of base
    /// tokens (which can be obtained from base_range).
    max_counter_tokens: u64,

    /// Timestamp at which the quote arrived, in nanoseconds since the Epoch.
    timestamp: u64,
}

impl Quote {
    /// Create a new quote from SCI and timestamp.
    ///
    /// # Arguments
    /// * `sci` - The SCI to add.
    /// * `timestamp` - The timestamp of the block containing the SCI. If not
    ///   provided, the system time is used.
    pub fn new(sci: SignedContingentInput, timestamp: Option<u64>) -> Result<Self, Error> {
        let timestamp = if let Some(timestamp) = timestamp {
            timestamp
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Error::Time)?
                .as_nanos()
                .try_into()
                .map_err(|_| Error::Time)?
        };

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

        let (counter_token_id, base_range, max_counter_tokens) = match (
            input_rules.required_outputs.len(),
            input_rules.partial_fill_outputs.len(),
        ) {
            (0, 0) => return Err(Error::UnsupportedSci("No required/partial outputs".into())),
            (1, 0) => {
                // Single required non-partial output
                (
                    TokenId::from(sci.required_output_amounts[0].token_id),
                    sci.pseudo_output_amount.value..=sci.pseudo_output_amount.value,
                    sci.required_output_amounts[0].value,
                )
            }
            (num_required_outputs @ (0 | 1), 1) => {
                // Single partial output or a single partial output + suspected change output
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount()?;
                let min_base_amount = input_rules.min_partial_fill_value;
                let mut max_base_amount = sci.pseudo_output_amount.value;

                // If we have a required output in addition to our partial output, we would
                // expect it to be a change output. We assume its change if it
                // is in the same token id as the base token, since it takes
                // away from the amount of base tokens available for consumption
                // by the fulfiller.
                if num_required_outputs == 1 {
                    if sci.required_output_amounts[0].token_id != base_token_id {
                        return Err(Error::UnsupportedSci(format!(
                        "Suspected required-change-output token id {} does not match partial output token id {}",
                        sci.required_output_amounts[0].token_id, amount.token_id
                    )));
                    }
                    max_base_amount = max_base_amount
                        .checked_sub(sci.required_output_amounts[0].value)
                        .ok_or_else(|| {
                            Error::UnsupportedSci(format!(
                                "max base amount {} is lower than required change {}",
                                max_base_amount, sci.required_output_amounts[0].value
                            ))
                        })?;
                }

                (
                    amount.token_id,
                    min_base_amount..=max_base_amount,
                    amount.value,
                )
            }
            _ => {
                return Err(Error::UnsupportedSci(format!(
                    "Unsupported number of required/partial outputs {}/{}",
                    input_rules.required_outputs.len(),
                    input_rules.partial_fill_outputs.len()
                )))
            }
        };

        let id = QuoteId::from(&sci);

        let pair = Pair {
            base_token_id,
            counter_token_id,
        };

        Ok(Self {
            sci,
            id,
            pair,
            base_range,
            max_counter_tokens,
            timestamp,
        })
    }

    /// Create a new quote by specifying the exact value for each field.
    pub fn new_from_fields(
        sci: SignedContingentInput,
        id: QuoteId,
        pair: Pair,
        base_range: RangeInclusive<u64>,
        max_counter_tokens: u64,
        timestamp: u64,
    ) -> Self {
        Self {
            sci,
            id,
            pair,
            base_range,
            max_counter_tokens,
            timestamp,
        }
    }

    /// Get underlying SCI.
    pub fn sci(&self) -> &SignedContingentInput {
        &self.sci
    }

    /// Get unique identifier.
    pub fn id(&self) -> &QuoteId {
        &self.id
    }

    /// Get the pair being traded by this quote.
    pub fn pair(&self) -> &Pair {
        &self.pair
    }

    /// Get the range of base tokens offered by this quote (the minimum and
    /// maximum amount of base token that can be obtained by fulfiling the
    /// quote).
    pub fn base_range(&self) -> &RangeInclusive<u64> {
        &self.base_range
    }

    /// Get the maximum amount of base tokens that can be provided by this
    /// quote.
    pub fn max_base_tokens(&self) -> u64 {
        *self.base_range.end()
    }

    /// Get the maximum amount of counter tokens required to completely use all
    /// the available base tokens
    pub fn max_counter_tokens(&self) -> u64 {
        self.max_counter_tokens
    }

    /// Get timestamp.
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    // Get the number of counter tokens we will need to provide in quote to consume
    // this SCI and receive a total of base_tokens back.
    pub fn counter_tokens_cost(&self, base_tokens: u64) -> Result<u64, Error> {
        if !self.base_range.contains(&base_tokens) {
            return Err(Error::InsufficientBaseTokens(base_tokens));
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
            (0, 0) => Err(Error::UnsupportedSci("No required/partial outputs".into())),

            (1, 0) => {
                // Single required non-partial output. This quote can only execute if are taking
                // the entire amount.
                // The assert here makes sense since we should only get here if base_tokens is a
                // range containing only self.sci.pseudo_output_amount.value
                assert!(base_tokens == self.sci.pseudo_output_amount.value);

                // In a non-partial swap, the fulfiller need to provide the amount specified in
                // the (only, for now) required output.
                Ok(self.sci.required_output_amounts[0].value)
            }

            (num_required_outputs @ (0 | 1), 1) => {
                // Single partial output or a single partial output + change amount.
                // The fact that the required output is treated as change has been verified when
                // the Quote was created.

                // The amount we are taking must be above the minimum fill value. It is expected
                // to be, since we checked base_range at the beginning.
                assert!(base_tokens >= input_rules.min_partial_fill_value);

                // The amount we are taking must be below the maximum available. It is expected
                // to be, since we checked base_range at the beginning.
                let mut max_available_amount = self.sci.pseudo_output_amount.value;
                if num_required_outputs == 1 {
                    assert!(max_available_amount > self.sci.required_output_amounts[0].value);
                    max_available_amount -= self.sci.required_output_amounts[0].value;
                };
                assert!(base_tokens <= max_available_amount);

                // The ratio being filled
                let fill_fraction_num: u128 = base_tokens as u128;
                let fill_fractions_denom = self.sci.pseudo_output_amount.value as u128;

                // Calculate the number of counter tokens we need to return as change to the
                // offerer of the SCI. It is calculated as a fraction of the partial fill
                // output.
                let (amount, _) = input_rules.partial_fill_outputs[0].reveal_amount()?;
                let num_128 = amount.value as u128 * fill_fraction_num;
                // Divide and round down
                Ok((num_128 / fill_fractions_denom) as u64)
            }

            _ => Err(Error::UnsupportedSci(format!(
                "Unsupported number of required/partial outputs {}/{}",
                input_rules.required_outputs.len(),
                input_rules.partial_fill_outputs.len()
            ))),
        }
    }
}

impl TryFrom<SignedContingentInput> for Quote {
    type Error = Error;

    fn try_from(sci: SignedContingentInput) -> Result<Self, Self::Error> {
        Self::new(sci, None)
    }
}

impl Deref for Quote {
    type Target = SignedContingentInput;

    fn deref(&self) -> &Self::Target {
        &self.sci
    }
}

impl Ord for Quote {
    fn cmp(&self, other: &Self) -> Ordering {
        // We sort quotes by the following, in this quote:
        // 1) The pair (so that quotes of the same pair are grouped together)
        // 2) The ratio of base to counter, putting quotes with a more favorable
        // exchange rate (to the fulfiller) first.
        // 3) Timestamp (so that older quotes are filled first)
        // 4) Quote id (in case of quotes where all the above were identical)

        // The rate is calculated as base / counter. We want to sort by:
        // (self_base / self_counter) > (other_base / other_counter)
        // Since we want to avoid division, we multiply both sides by the denominators
        // and get: self_base * other_counter > other_base * self_counter
        let self_rate = other.max_base_tokens() as u128 * self.max_counter_tokens() as u128;
        let other_rate = self.max_base_tokens() as u128 * other.max_counter_tokens() as u128;

        let k1 = (&self.pair, self_rate, &self.timestamp, &self.id);
        let k2 = (&other.pair, other_rate, &other.timestamp, &other.id);

        k1.cmp(&k2)
    }
}

impl PartialOrd for Quote {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ directory since we want to be able to
    // re-use some test code between implementations and that seems to be the
    // way to make Rust do that.
}
