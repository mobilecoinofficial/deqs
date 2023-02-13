// Copyright (c) 2023 MobileCoin Inc.

use super::rpc::{Request, Response};
use mc_util_metrics::{
    Collector, Desc, HistogramOpts, HistogramVec, IntCounterVec, MetricFamily, Opts,
};
use prometheus::HistogramTimer;

lazy_static::lazy_static! {
    pub static ref P2P_RPC_METRICS: P2PRpcMetrics = P2PRpcMetrics::default();
}

/// `P2PRpcMetrics` is a metric [`Collector`](prometheus::core::Collector) to
/// capture key metrics about a P2P RPC calls
///
/// This is based on `mobilecoin/util/metrics/src/service_metrics.rs`.
#[derive(Clone)]
pub struct P2PRpcMetrics {
    /// Count of requests received by each RPC method tracked
    num_req: IntCounterVec,

    /// Count of success responses for each RPC method tracked
    num_success: IntCounterVec,

    /// Count of error responses for each RPC method tracked
    num_error: IntCounterVec,

    /// Duration of RPC method calls tracked
    duration: HistogramVec,
}
impl Default for P2PRpcMetrics {
    fn default() -> Self {
        P2PRpcMetrics::new_and_registered("deqs_server_p2p_rpc")
    }
}

impl P2PRpcMetrics {
    /// Create a default constructor that initializes all metrics
    pub fn new<S: Into<String>>(name: S) -> P2PRpcMetrics {
        let name_str = name.into();

        Self {
            num_req: IntCounterVec::new(
                Opts::new(format!("{name_str}_num_req"), "Number of requests"),
                &["method"],
            )
            .unwrap(),
            num_error: IntCounterVec::new(
                Opts::new(format!("{name_str}_num_error"), "Number of errors"),
                &["method"],
            )
            .unwrap(),
            num_success: IntCounterVec::new(
                Opts::new(format!("{name_str}_num_success"), "Number of successes"),
                &["method"],
            )
            .unwrap(),
            duration: HistogramVec::new(
                HistogramOpts::new(
                    format!("{name_str}_duration"),
                    "Duration for a request, in units of time",
                ),
                &["method"],
            )
            .unwrap(),
        }
    }
}

impl P2PRpcMetrics {
    /// Register Prometheus metrics family
    pub fn new_and_registered<S: Into<String>>(name: S) -> Self {
        let svc = Self::new(name);
        let _res = prometheus::register(Box::new(svc.clone()));
        svc
    }

    /// Takes the RpcContext used during a gRPC method call to get the method
    /// name and increments counters tracking the number of calls to and
    /// returns a counter to track the duration of the method
    pub fn req(&self, req: &Request) -> (HistogramTimer, &'static str) {
        let method_name = req.metrics_method_name();

        self.num_req.with_label_values(&[method_name]).inc();

        (
            self.duration
                .with_label_values(&[method_name])
                .start_timer(),
            method_name,
        )
    }

    /// Takes the RpcContext used during a gRPC method call to get the method
    /// name and increments an error counter if the method resulted in an
    /// error
    pub fn resp(&self, method_name: &'static str, resp: &Response) {
        if let Response::Error(_) = resp {
            self.num_error.with_label_values(&[method_name]).inc_by(1);
        } else {
            self.num_success.with_label_values(&[method_name]).inc_by(1);
        }
    }
}

impl Collector for P2PRpcMetrics {
    /// Collect metric descriptions for Prometheus
    fn desc(&self) -> Vec<&Desc> {
        // order: num_req, num_error, duration
        vec![
            self.num_req.desc(),
            self.num_error.desc(),
            self.num_success.desc(),
            self.duration.desc(),
        ]
        .into_iter()
        .map(|m| m[0])
        .collect()
    }

    /// Collect Prometheus metrics
    fn collect(&self) -> Vec<MetricFamily> {
        // families
        let vs = vec![
            self.num_req.collect(),
            self.num_error.collect(),
            self.num_success.collect(),
            self.duration.collect(),
        ];

        vs.into_iter().fold(vec![], |mut l, v| {
            l.extend(v);
            l
        })
    }
}
