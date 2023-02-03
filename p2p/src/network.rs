// Copyright (c) 2023 MobileCoin Inc.

use libp2p::{gossipsub::GossipsubMessage, request_response::ResponseChannel, PeerId};
use tokio::sync::mpsc;

use crate::{client::Client, network_event_loop::NetworkEventLoop, RpcRequest, RpcResponse};

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

/// A collection of objects that together form an interface to a p2p network.
pub struct Network<REQ: RpcRequest, RESP: RpcResponse> {
    /// The event loop that processes network and client events.
    pub event_loop: NetworkEventLoop<REQ, RESP>,

    /// A client for interacting with the network.
    pub client: Client<REQ, RESP>,

    /// Channel for receiving asynchronous network events.
    pub events: mpsc::UnboundedReceiver<NetworkEvent<REQ, RESP>>,
}
