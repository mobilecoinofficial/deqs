// Copyright (c) 2023 MobileCoin Inc.

use crate::{mini_wallet::MiniWallet, Error, LiquidityBot};
use mc_util_metrics::{IntGauge, IntGaugeVec, OpMetrics, Opts};
use std::time::Duration;

/// Frequency at which we update metrics.
pub const METRICS_POLL_INTERVAL: Duration = Duration::from_secs(1);

// Metrics to track:
// Number of pending tx outs (grouped by token id)
// Value of pending tx outs (grouped by token id)
// Number of listed tx outs (grouped by token id)
// Value of listed tx outs (grouped by token id)

lazy_static::lazy_static! {
    /// Counters metrics tracker (for metrics that are not grouped by token id)
    pub static ref OP_COUNTERS: OpMetrics = OpMetrics::new_and_registered("deqs_liquidity_bot");

    /// Per-token metrics tracker
    pub static ref PER_TOKEN_METRICS: PerTokenMetrics = PerTokenMetrics::new().expect("Failed to create per-token metrics");

   /// Next block index to scan
   pub static ref NEXT_BLOCK_INDEX: IntGauge = OP_COUNTERS.gauge("next_block_index");

}

/// Update periodic metrics.
pub async fn update_periodic_metrics(wallet: &MiniWallet, liquidity_bot: &LiquidityBot) {
    NEXT_BLOCK_INDEX.set(wallet.next_block_index() as i64);
    PER_TOKEN_METRICS
        .update_metrics_from_liquidity_bot(liquidity_bot)
        .await;
}

pub struct PerTokenMetrics {
    /// Number of pending tx outs
    pending_tx_outs_gauge: IntGaugeVec,
    // /// Total value of pending tx outs
    // pending_tx_outs_value_gauge: IntGaugeVec,

    // /// Number of listed tx outs
    // listed_tx_outs_gauge: IntGaugeVec,

    // /// Total value of listed tx outs
    // listed_tx_outs_value_gauge: IntGaugeVec,
}

impl PerTokenMetrics {
    pub fn new() -> Result<Self, Error> {
        let pending_tx_outs_gauge = IntGaugeVec::new(
            Opts::new(
                "deqs_liquidity_bot_pending_tx_outs",
                "Number of pending tx outs",
            ),
            &["token_id"],
        )?;

        prometheus::register(Box::new(pending_tx_outs_gauge.clone()))?;

        Ok(Self {
            pending_tx_outs_gauge,
        })
    }

    pub async fn update_metrics_from_liquidity_bot(&self, liquidity_bot: &LiquidityBot) {
        let stats = liquidity_bot.stats().await;

        for (token_id, value) in stats.num_pending_tx_outs_by_token_id.iter() {
            self.pending_tx_outs_gauge
                .with_label_values(&[&token_id.to_string()])
                .set(*value as i64);
        }
    }
}
