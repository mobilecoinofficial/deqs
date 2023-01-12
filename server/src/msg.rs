// Copyright (c) 2023 MobileCoin Inc.

/// Data type for encapsulating messages sent over the internal message bus
#[derive(Clone, Debug)]
pub enum Msg {
    /// SCI added to order book
    SciOrderAdded
}
