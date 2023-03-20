// Copyright (c) 2023 MobileCoin Inc.

use deqs_liquidity_bot_api::{
    liquidity_bot::{
        GetListedTxOutsRequest, GetListedTxOutsResponse, GetPendingTxOutsRequest,
        GetPendingTxOutsResponse,
    },
    liquidity_bot_grpc::{create_deqs_liquidity_bot_admin_api, DeqsLiquidityBotAdminApi},
};
use futures::executor::block_on;
use grpcio::{RpcContext, RpcStatus, Service, UnarySink};
use mc_common::logger::{scoped_global_logger, Logger};
use mc_util_grpc::{rpc_logger, send_result};

use crate::LiquidityBotInterface;

/// GRPC Admin Service.
#[derive(Clone)]
pub struct AdminService {
    liquidity_bot: LiquidityBotInterface,
    logger: Logger,
}

impl AdminService {
    pub fn new(liquidity_bot: LiquidityBotInterface, logger: Logger) -> Self {
        Self {
            liquidity_bot,
            logger,
        }
    }

    /// Convert into a grpc service
    pub fn into_service(self) -> Service {
        create_deqs_liquidity_bot_admin_api(self)
    }

    async fn get_pending_tx_outs_impl(&self) -> Result<GetPendingTxOutsResponse, RpcStatus> {
        let pending_tx_outs = self.liquidity_bot.pending_tx_outs().await;
        Ok(GetPendingTxOutsResponse {
            pending_tx_outs: pending_tx_outs.iter().map(|tx_out| tx_out.into()).collect(),
            ..Default::default()
        })
    }

    async fn get_listed_tx_outs_impl(&self) -> Result<GetListedTxOutsResponse, RpcStatus> {
        let listed_tx_outs = self.liquidity_bot.listed_tx_outs().await;
        Ok(GetListedTxOutsResponse {
            listed_tx_outs: listed_tx_outs.iter().map(|tx_out| tx_out.into()).collect(),
            ..Default::default()
        })
    }


}

impl DeqsLiquidityBotAdminApi for AdminService {
    fn get_pending_tx_outs(
        &mut self,
        ctx: RpcContext,
        _req: GetPendingTxOutsRequest,
        sink: UnarySink<GetPendingTxOutsResponse>,
    ) {
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, block_on(self.get_pending_tx_outs_impl()), logger)
        })
    }

    fn get_listed_tx_outs(
        &mut self,
        ctx: RpcContext,
        _req: GetListedTxOutsRequest,
        sink: UnarySink<GetListedTxOutsResponse>,
    ) {
        scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, block_on(self.get_listed_tx_outs_impl()), logger)
        })
    }
}
