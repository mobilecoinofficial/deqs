// Copyright (c) 2023 MobileCoin Inc.

//! Peer to peer networking.
//! This is based on https://github.com/libp2p/rust-libp2p/pull/3150/files

mod behaviour;
mod client;
mod error;
mod network;
mod network_builder;
mod network_event_loop;
mod rpc;

pub use behaviour::{Behaviour, OutEvent};
pub use client::{Client, Error as ClientError};
pub use error::Error;
pub use network::{Network, NetworkEvent};
pub use network_builder::NetworkBuilder;
pub use network_event_loop::NetworkEventLoopHandle;
pub use rpc::{RpcRequest, RpcResponse};
