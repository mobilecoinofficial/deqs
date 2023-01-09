// Copyright (c) 2023 MobileCoin Inc.

// TODO: Adding/removing is currently very inefficient.

use crate::OrderBook;
use displaydoc::Display;
use mc_crypto_digestible::{Digestible, MerlinTranscript};
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::{SignedContingentInput, SignedContingentInputError};
use mc_transaction_types::TokenId;
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    hash::{Hash, Hasher},
    ops::Deref,
    sync::{Arc, Mutex, PoisonError},
    time::{SystemTime, SystemTimeError},
};

/// A newtype around SCIs that sort them by (cost, quantity available, time of
/// creation, hash)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PricedSci {
    /// The actual SCI
    sci: SignedContingentInput,

    /// Key to sort by
    sort_key: (u64, u64, u128, [u8; 32]),

    /// Unique identifier
    id: [u8; 32],
}

impl Hash for PricedSci {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PricedSci {
    fn sort_key(&self) -> &(u64, u64, u128, [u8; 32]) {
        &self.sort_key
    }
}

impl Ord for PricedSci {
    fn cmp(&self, other: &Self) -> Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for PricedSci {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl TryFrom<SignedContingentInput> for PricedSci {
    type Error = Error;

    fn try_from(sci: SignedContingentInput) -> Result<Self, Self::Error> {
        // TODO: This assumes the first output is the one that specifies how much to pay
        // the offerer
        let digest = sci.digest32::<MerlinTranscript>(b"deqs-priced-sci");
        let sort_key = (
            sci.required_output_amounts[0].value,
            sci.pseudo_output_amount.value,
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_nanos(),
            digest,
        );

        Ok(Self {
            sci,
            sort_key,
            id: digest,
        })
    }
}

impl Deref for PricedSci {
    type Target = SignedContingentInput;

    fn deref(&self) -> &Self::Target {
        &self.sci
    }
}

// TODO: Maybe this should be shared between OrderBook implementations
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Pair {
    /// The token id being offered.
    offered_token_id: TokenId,

    /// The token id that needs to be paid to satisfy the offering.
    /// (The SCI is "priced" with this token id)
    priced_token_id: TokenId,
}

impl From<&SignedContingentInput> for Pair {
    fn from(sci: &SignedContingentInput) -> Self {
        // TODO: This assumes the first output is the one that specifies what we need to
        // pay the offerer
        Self {
            offered_token_id: TokenId::from(sci.pseudo_output_amount.token_id),
            priced_token_id: TokenId::from(sci.required_output_amounts[0].token_id),
        }
    }
}

/// A naive in-memory order book implementation
#[derive(Clone, Debug)]
pub struct InMemoryOrderBook {
    /// A map of (offered token ID, requested token ID) to a sorted list of SCIs
    /// that offer them
    scis: Arc<Mutex<HashMap<Pair, BTreeSet<PricedSci>>>>,
}

impl InMemoryOrderBook {
    pub fn new() -> Self {
        Self {
            scis: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl OrderBook for InMemoryOrderBook {
    type Error = Error;

    /// SCIs are identified by their digest
    type SciId = [u8; 32];

    fn add_sci(&self, sci: SignedContingentInput) -> Result<(), Self::Error> {
        // The SCI must be valid.
        sci.validate()?;

        // TODO - Sanity - we currently expect the SCI to contain only a single required
        // output Future version might require another output for paying fees to
        // the DEQS
        if sci.required_output_amounts.len() != 1 {
            return Err(Error::IncorrectNumberOfOutputs);
        }

        // Try adding the SCI.
        let priced_sci = PricedSci::try_from(sci)?;
        let pair = Pair::from(&*priced_sci);

        let mut scis = self.scis.lock()?;
        let entry = scis.entry(pair).or_insert_with(Default::default);

        if !entry.insert(priced_sci) {
            Err(Error::AlreadyExists)
        } else {
            Ok(())
        }
    }

    fn remove_single_sci(&self, sci_id: &Self::SciId) -> Result<(), Self::Error> {
        let mut scis = self.scis.lock()?;

        for entries in scis.values_mut() {
            if let Some(entry) = entries.iter().find(|entry| entry.id == *sci_id).cloned() {
                entries.remove(&entry);
                return Ok(());
            }
        }

        Err(Error::SciNotFound)
    }

    fn remove_scis(&self, key_image: &KeyImage) -> Result<(), Self::Error> {
        let mut scis = self.scis.lock()?;

        for entries in scis.values_mut() {
            entries.retain(|entry| entry.key_image() != *key_image);
        }

        Ok(())
    }

    fn get_scis(
        &self,
        offered_token_id: TokenId,
        priced_token_id: TokenId,
        min_payout: Option<u64>,
        max_cost: Option<u64>,
    ) -> Result<HashMap<Self::SciId, SignedContingentInput>, Self::Error> {
        let pair = Pair {
            offered_token_id,
            priced_token_id,
        };
        let scis = self.scis.lock()?;
        if let Some(entries) = scis.get(&pair) {
            todo!()
        } else {
            Ok(Default::default())
        }
    }
}

/// Error data type
#[derive(Debug, Display)]
pub enum Error {
    /// SCI: {0}
    Sci(SignedContingentInputError),

    /// System time: {0}
    SystemTime(SystemTimeError),

    /// Number of outputs is incorrect
    IncorrectNumberOfOutputs,

    /// Mutex poisoned
    MutexPoisoned,

    /// SCI already exists in book
    AlreadyExists,

    /// SCI not found
    SciNotFound,
}

impl From<SignedContingentInputError> for Error {
    fn from(src: SignedContingentInputError) -> Self {
        Self::Sci(src)
    }
}

impl From<SystemTimeError> for Error {
    fn from(src: SystemTimeError) -> Self {
        Self::SystemTime(src)
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(_src: PoisonError<T>) -> Self {
        Error::MutexPoisoned
    }
}
