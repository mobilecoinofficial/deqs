// Copyright (c) 2023 MobileCoin Inc.

use crate::{
    p2p::{RpcError, RpcQuoteBookError},
    Error,
};
use deqs_p2p::{libp2p::PeerId, Client};
use deqs_quote_book::{Quote, QuoteId};
use mc_common::logger::{log, Logger};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Request {
    GetAllQuoteIds,
    GetQuoteById(QuoteId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Response {
    AllQuoteIds(Vec<QuoteId>),
    MaybeQuote(Option<Quote>),
    Error(RpcError),
}

#[derive(Clone)]
pub struct RpcClient {
    client: Client<Request, Response>,
    logger: Logger,
}

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

    pub async fn get_quote_by_id(
        &mut self,
        peer_id: PeerId,
        quote_id: QuoteId,
    ) -> Result<Option<Quote>, Error> {
        match self
            .client
            .rpc_request(peer_id, Request::GetQuoteById(quote_id))
            .await
        {
            Ok(Response::MaybeQuote(quote)) => Ok(quote),

            Ok(Response::Error(RpcError::QuoteBook(RpcQuoteBookError::QuoteNotFound))) => Ok(None),

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
