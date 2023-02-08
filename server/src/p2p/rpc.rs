// Copyright (c) 2023 MobileCoin Inc.

use crate::{p2p::RpcError, Error};
use deqs_p2p::{libp2p::PeerId, Client};
use deqs_quote_book::{Quote, QuoteId};
use mc_common::logger::{log, Logger};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Request {
    GetAllQuoteIds,
    GetQuotesById(Vec<QuoteId>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Response {
    AllQuoteIds(Vec<QuoteId>),
    MaybeQuotes(Vec<Result<Option<Quote>, RpcError>>),
    Error(RpcError),
}

#[derive(Clone)]
pub struct RpcClient {
    client: Client<Request, Response>,
    logger: Logger,
}

/// A wrapper around the p2p client that handles serialization and
/// deserialization of requests and responses
impl RpcClient {
    pub fn new(client: Client<Request, Response>, logger: Logger) -> Self {
        Self { client, logger }
    }

    pub async fn get_all_quote_ids(&mut self, peer_id: PeerId) -> Result<Vec<QuoteId>, Error> {
        match self
            .client
            .rpc_request(peer_id, Request::GetAllQuoteIds)
            .await
        {
            Ok(Response::AllQuoteIds(quote_ids)) => Ok(quote_ids),

            Ok(Response::Error(err)) => {
                log::info!(self.logger, "Received error from peer {}: {}", peer_id, err);
                Err(err.into())
            }

            Ok(response) => {
                log::warn!(
                    self.logger,
                    "Received unexpected response from peer{} : {:?}",
                    peer_id,
                    response
                );
                Err(Error::P2PRpc(RpcError::UnexpectedResponse))
            }
            Err(err) => {
                log::info!(
                    self.logger,
                    "Failed to get quote from peer {}: {}",
                    peer_id,
                    err
                );
                Err(err.into())
            }
        }
    }

    pub async fn get_quotes_by_id(
        &mut self,
        peer_id: PeerId,
        quote_ids: Vec<QuoteId>,
    ) -> Result<Vec<Result<Option<Quote>, RpcError>>, Error> {
        let quote_ids_len = quote_ids.len();

        match self
            .client
            .rpc_request(peer_id, Request::GetQuotesById(quote_ids))
            .await
        {
            Ok(Response::MaybeQuotes(quotes)) => {
                if quote_ids_len == quotes.len() {
                    Ok(quotes)
                } else {
                    log::warn!(
                        self.logger,
                        "Received unexpected number of quotes from {}: expected {}, got {}",
                        peer_id,
                        quote_ids_len,
                        quotes.len()
                    );
                    Err(Error::P2PRpc(RpcError::UnexpectedResponse))
                }
            }

            Ok(Response::Error(err)) => {
                log::info!(self.logger, "Received error from peer {}: {}", peer_id, err);
                Err(err.into())
            }

            Ok(response) => {
                log::warn!(
                    self.logger,
                    "Received unexpected response from peer{} : {:?}",
                    peer_id,
                    response
                );
                Err(Error::P2PRpc(RpcError::UnexpectedResponse))
            }
            Err(err) => {
                log::info!(
                    self.logger,
                    "Failed to get quote from peer {}: {}",
                    peer_id,
                    err
                );
                Err(err.into())
            }
        }
    }
}
