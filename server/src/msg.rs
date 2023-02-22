// Copyright (c) 2023 MobileCoin Inc.

use deqs_quote_book_api::Quote;

/// The source of a message on the message bus.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MsgSource {
    /// TODO
    Gossip,

    /// TODO
    P2pSync,

    /// TODO
    GrpcClient,
}

/// Data type for encapsulating messages sent over the internal message bus
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Msg {
    /// SCI added to quote book
    SciQuoteAdded(Quote, MsgSource),

    /// SCI removed from the quote book
    SciQuoteRemoved(Quote, MsgSource),
}
