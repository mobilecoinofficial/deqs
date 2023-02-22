// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, NotifyingQuoteBook, P2P};
use deqs_quote_book_api::QuoteBook;
use mc_util_metrics::{IntGauge, OpMetrics, ServiceMetrics};
use std::time::Duration;

/// Frequency at which we update metrics.
pub const METRICS_POLL_INTERVAL: Duration = Duration::from_secs(1);

lazy_static::lazy_static! {
   /// GRPC metrics tracker.
   pub static ref SVC_COUNTERS: ServiceMetrics = ServiceMetrics::new_and_registered("deqs_server");

    /// Counters metrics tracker.
    pub static ref OP_COUNTERS: OpMetrics = OpMetrics::new_and_registered("deqs_server");

   /// Number of SCIs currently in the quote book, updated every METRICS_POLL_INTERVAL.
   pub static ref SCI_COUNT: IntGauge = OP_COUNTERS.gauge("sci_count");

   /// Number of currently connected p2p peers.
   pub static ref P2P_NUM_CONNECTED_PEERS: IntGauge = OP_COUNTERS.gauge("p2p_num_connected_peers");
}

/// Update periodic metrics.
pub async fn update_periodic_metrics<QB: QuoteBook>(
    quote_book: &NotifyingQuoteBook<QB>,
    p2p: &P2P<QB>,
) -> Result<(), Error> {
    SCI_COUNT.set(quote_book.num_scis()? as i64);
    P2P_NUM_CONNECTED_PEERS.set(p2p.connected_peers().await?.len() as i64);

    Ok(())
}
