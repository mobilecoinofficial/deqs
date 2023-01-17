// Copyright (c) 2023 MobileCoin Inc.

use displaydoc::Display;

/// Error data type
#[derive(Debug, Display)]
pub enum Error {
    /// GRPC: {0}
    Grpc(grpcio::Error),
}

impl From<grpcio::Error> for Error {
    fn from(src: grpcio::Error) -> Self {
        Self::Grpc(src)
    }
}
