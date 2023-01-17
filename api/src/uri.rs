// Copyright (c) 2023 MobileCoin Inc.

use mc_util_uri::{Uri, UriScheme};

/// Fog View Uri Scheme
#[derive(Debug, Hash, Ord, PartialOrd, Eq, PartialEq, Clone)]
pub struct DeqsClientScheme {}

impl UriScheme for DeqsClientScheme {
    /// The part before the '://' of a URL.
    const SCHEME_SECURE: &'static str = "deqs";
    const SCHEME_INSECURE: &'static str = "insecure-deqs";

    /// Default port numbers
    const DEFAULT_SECURE_PORT: u16 = 443;
    const DEFAULT_INSECURE_PORT: u16 = 7000;
}

/// Uri used when talking to a DEQS service, with the right default ports and
/// scheme.
pub type DeqsClientUri = Uri<DeqsClientScheme>;
