// Copyright (c) 2023 MobileCoin Inc.

use async_trait::async_trait;
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};
use libp2p::{
    core::upgrade::{read_length_prefixed, write_length_prefixed},
    request_response::{ProtocolName, RequestResponseCodec},
};
use serde::{de::DeserializeOwned, Serialize};
use std::{fmt::Debug, io};

pub trait RpcRequest:
    Clone + Debug + DeserializeOwned + Eq + PartialEq + Serialize + Send + 'static
{
}
impl<T> RpcRequest for T where
    T: Clone + Debug + DeserializeOwned + Eq + PartialEq + Serialize + Send + 'static
{
}

pub trait RpcResponse:
    Clone + Debug + DeserializeOwned + Eq + PartialEq + Serialize + Send + 'static
{
}
impl<T> RpcResponse for T where
    T: Clone + Debug + DeserializeOwned + Eq + PartialEq + Serialize + Send + 'static
{
}

#[derive(Debug, Clone)]
pub struct RpcProtocol;

#[derive(Clone)]
pub struct RpcCodec<REQ: RpcRequest, RESP: RpcResponse> {
    _req: std::marker::PhantomData<REQ>,
    _resp: std::marker::PhantomData<RESP>,
}
impl<REQ: RpcRequest, RESP: RpcResponse> Default for RpcCodec<REQ, RESP> {
    fn default() -> Self {
        Self {
            _req: std::marker::PhantomData,
            _resp: std::marker::PhantomData,
        }
    }
}

impl ProtocolName for RpcProtocol {
    fn protocol_name(&self) -> &[u8] {
        "/mc/deqs/p2p/rpc".as_bytes()
    }
}

#[async_trait]
impl<REQ: RpcRequest, RESP: RpcResponse> RequestResponseCodec for RpcCodec<REQ, RESP> {
    type Protocol = RpcProtocol;
    type Request = REQ;
    type Response = RESP;

    async fn read_request<T>(&mut self, _: &RpcProtocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 1_000_000).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        mc_util_serial::deserialize(&vec)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.to_string()))
    }

    async fn read_response<T>(&mut self, _: &RpcProtocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 500_000_000).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        mc_util_serial::deserialize(&vec)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.to_string()))
    }

    async fn write_request<T>(&mut self, _: &RpcProtocol, io: &mut T, req: REQ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let data = mc_util_serial::serialize(&req).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to serialize request: {err}"),
            )
        })?;
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }

    async fn write_response<T>(&mut self, _: &RpcProtocol, io: &mut T, resp: RESP) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let data = mc_util_serial::serialize(&resp).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to serialize response: {err}"),
            )
        })?;

        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }
}
