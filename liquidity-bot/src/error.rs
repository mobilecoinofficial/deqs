// Copyright (c) 2023 MobileCoin Inc.

use std::io;

use displaydoc::Display;
use mc_crypto_keys::KeyError;
use mc_ledger_db::Error as LedgerDbError;
use mc_transaction_builder::{SignedContingentInputBuilderError, TxBuilderError};
use mc_transaction_core::{AmountError, RevealedTxOutError, TokenId, TxOutConversionError};
use mc_util_serial::{decode::Error as DeserializeError, encode::Error as SerializeError};
use serde_json::Error as JsonError;

#[derive(Debug, Display)]
pub enum Error {
    /// Ledger Db: {0}
    LedgerDb(LedgerDbError),

    /// Tx Builder: {0}
    TxBuilderError(TxBuilderError),

    /// Signed Contingent Input Builder: {0}
    SignedContingentInputBuilder(SignedContingentInputBuilderError),

    /// Amount: {0}
    Amount(AmountError),

    /// TxOut conversion: {0}
    TxOutConversion(TxOutConversionError),

    /// Crypto key: {0}
    Key(KeyError),

    /// IO: {0}
    IO(io::Error),

    /// Json: {0}
    Json(JsonError),

    /// GRPC: {0}
    Grpc(grpcio::Error),

    /// Invalid GRPC response: {0}
    InvalidGrpcResponse(String),

    /// Api Conversion: {0}
    ApiConversion(deqs_api::ConversionError),

    /// Unknown token id: {0}
    UnknownTokenId(TokenId),

    /// Decimal conversion: {0}
    DecimalConversion(String),

    /// Deserialize: {0}
    Deserialize(DeserializeError),

    /// Serialize: {0}
    Serialize(SerializeError),

    /// Prometheus: {0}
    Prometheus(prometheus::Error),

    /// RevealedTxOut: {0}
    RevealedTxOutError(RevealedTxOutError),
}
impl From<LedgerDbError> for Error {
    fn from(src: LedgerDbError) -> Self {
        Self::LedgerDb(src)
    }
}
impl From<TxBuilderError> for Error {
    fn from(src: TxBuilderError) -> Self {
        Self::TxBuilderError(src)
    }
}
impl From<SignedContingentInputBuilderError> for Error {
    fn from(src: SignedContingentInputBuilderError) -> Self {
        Self::SignedContingentInputBuilder(src)
    }
}
impl From<AmountError> for Error {
    fn from(src: AmountError) -> Self {
        Self::Amount(src)
    }
}
impl From<TxOutConversionError> for Error {
    fn from(src: TxOutConversionError) -> Self {
        Self::TxOutConversion(src)
    }
}
impl From<KeyError> for Error {
    fn from(src: KeyError) -> Self {
        Self::Key(src)
    }
}
impl From<io::Error> for Error {
    fn from(src: io::Error) -> Self {
        Self::IO(src)
    }
}
impl From<JsonError> for Error {
    fn from(src: JsonError) -> Self {
        Self::Json(src)
    }
}
impl From<grpcio::Error> for Error {
    fn from(src: grpcio::Error) -> Self {
        Self::Grpc(src)
    }
}
impl From<deqs_api::ConversionError> for Error {
    fn from(src: deqs_api::ConversionError) -> Self {
        Self::ApiConversion(src)
    }
}
impl From<DeserializeError> for Error {
    fn from(src: DeserializeError) -> Self {
        Self::Deserialize(src)
    }
}
impl From<SerializeError> for Error {
    fn from(src: SerializeError) -> Self {
        Self::Serialize(src)
    }
}
impl From<prometheus::Error> for Error {
    fn from(src: prometheus::Error) -> Self {
        Self::Prometheus(src)
    }
}
impl From<RevealedTxOutError> for Error {
    fn from(src: RevealedTxOutError) -> Self {
        Self::RevealedTxOutError(src)
    }
}
