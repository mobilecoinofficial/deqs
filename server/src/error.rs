// Copyright (c) 2023 MobileCoin Inc.

use crate::p2p::RpcError;
use displaydoc::Display;
use mc_util_serial::{decode::Error as DecodeError, encode::Error as EncodeError};

/// Error data type
#[derive(Debug, Display)]
pub enum Error {
    /// GRPC: {0}
    Grpc(grpcio::Error),

    /// Decode: {0}
    Decode(DecodeError),

    /// Encode: {0}
    Encode(EncodeError),

    /// P2P Client: {0}
    P2PClient(deqs_p2p::ClientError),

    /// P2P: {0}
    P2P(deqs_p2p::Error),

    /// P2P RPC: {0}
    P2PRpc(RpcError),

    /// Quote book: {0}
    QuoteBook(deqs_quote_book::Error),
}

impl From<grpcio::Error> for Error {
    fn from(src: grpcio::Error) -> Self {
        Self::Grpc(src)
    }
}

impl From<DecodeError> for Error {
    fn from(src: DecodeError) -> Self {
        Self::Decode(src)
    }
}

impl From<EncodeError> for Error {
    fn from(src: EncodeError) -> Self {
        Self::Encode(src)
    }
}

impl From<deqs_p2p::ClientError> for Error {
    fn from(src: deqs_p2p::ClientError) -> Self {
        Self::P2PClient(src)
    }
}

impl From<deqs_p2p::Error> for Error {
    fn from(src: deqs_p2p::Error) -> Self {
        Self::P2P(src)
    }
}

impl From<RpcError> for Error {
    fn from(src: RpcError) -> Self {
        Self::P2PRpc(src)
    }
}

impl From<deqs_quote_book::Error> for Error {
    fn from(src: deqs_quote_book::Error) -> Self {
        Self::QuoteBook(src)
    }
}

impl std::error::Error for Error {}
