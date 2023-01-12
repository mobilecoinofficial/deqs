// Copyright (c) 2023 MobileCoin Inc.

use deqs_order_book::{Order, OrderId};

/// Data type for encapsulating messages sent over the internal message bus
#[derive(Clone, Debug)]
pub enum Msg {
    /// SCI added to order book
    SciOrderAdded(Order),

    /// SCI removed from the order book
    SciOrderRemoved(OrderId),

    // TODO
    V1,
}
