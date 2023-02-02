// Copyright (c) 2023 MobileCoin Inc.

use libp2p::{gossipsub::GossipsubMessage, request_response::ResponseChannel, PeerId};
use tokio::sync::mpsc;

use crate::{RpcRequest, RpcResponse, network_event_loop::NetworkEventLoop, client::Client};

/// An asynchronous event that can be received from the network.
#[derive(Debug)]
pub enum NetworkEvent<REQ: RpcRequest, RESP: RpcResponse> {
    /// Incoming RPC request
    RpcRequest {
        request: REQ,
        channel: ResponseChannel<RESP>,
    },

    /// Connection established with a peer.
    ConnectionEstablished { peer_id: PeerId },

    /// Gossip message received.
    GossipMessage { message: GossipsubMessage },
}

/// An interface to a p2p network.
pub struct Network<REQ: RpcRequest, RESP: RpcResponse> {
    pub event_loop: NetworkEventLoop<REQ, RESP>,
    pub client: Client<REQ, RESP>,
    pub events: mpsc::UnboundedReceiver<NetworkEvent<REQ, RESP>>,
}