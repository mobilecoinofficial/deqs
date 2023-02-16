// Copyright (c) 2023 MobileCoin Inc.

use crate::{Quote, QuoteId};

/// Data type for encapsulating messages sent over the internal message bus
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Msg {
    /// SCI added to quote book
    SciQuoteAdded(Quote),

    /// SCI removed from the quote book
    SciQuoteRemoved(QuoteId),
}
