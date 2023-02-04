// Copyright (c) 2023 MobileCoin Inc.

use mc_util_metrics::ServiceMetrics;

lazy_static::lazy_static! {
   pub static ref SVC_COUNTERS: ServiceMetrics = ServiceMetrics::new_and_registered("deqs_server");
}
