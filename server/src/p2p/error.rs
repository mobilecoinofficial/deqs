// Copyright (c) 2023 MobileCoin Inc.

use displaydoc::Display;
use libp2p::{noise::NoiseError, TransportError};
use libp2p_swarm::DialError;
use std::io::Error as IoError;

/// Error data type
#[derive(Debug, Display)]
pub enum Error {
    /// Gossipsub build error: {0}
    GossipsubBuild(&'static str),

    /// Gossipsub new error: {0}
    GossipsubNew(&'static str),

    /// Io error: {0}
    Io(IoError),

    /// Noise: {0}
    Noise(NoiseError),

    /// Dial: {0}
    Dial(DialError),

    /// Bootstrap: {0}
    Bootstrap(String),

    /// Transport: {0}
    Transport(TransportError<IoError>),
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Self::Io(e)
    }
}

impl From<NoiseError> for Error {
    fn from(e: NoiseError) -> Self {
        Self::Noise(e)
    }
}

impl From<DialError> for Error {
    fn from(e: DialError) -> Self {
        Self::Dial(e)
    }
}

impl From<TransportError<IoError>> for Error {
    fn from(e: TransportError<IoError>) -> Self {
        Self::Transport(e)
    }
}

impl std::error::Error for Error {}
