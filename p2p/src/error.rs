// Copyright (c) 2023 MobileCoin Inc.

use crate::ClientError;
use displaydoc::Display;
use libp2p::{
    kad::store::Error as KadStoreError, multiaddr::Error as MultiaddrError, noise::NoiseError,
    Multiaddr, TransportError,
};
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

    /// Multihash: {0}
    Multihash(String),

    /** Invalid peer address {0}, expected last component to be p2p multihash
     * and instead got {1}
     */
    InvalidPeerAddress(Multiaddr, String),

    /// Multiaddr: {0}
    Multiaddr(MultiaddrError),

    /// Kademlia store: {0}
    KadStore(KadStoreError),

    /// Client: {0}
    Client(ClientError),
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

impl From<MultiaddrError> for Error {
    fn from(e: MultiaddrError) -> Self {
        Self::Multiaddr(e)
    }
}

impl From<KadStoreError> for Error {
    fn from(e: KadStoreError) -> Self {
        Self::KadStore(e)
    }
}

impl From<ClientError> for Error {
    fn from(e: ClientError) -> Self {
        Self::Client(e)
    }
}

impl std::error::Error for Error {}
