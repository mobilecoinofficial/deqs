// Copyright (c) 2023 MobileCoin Inc.

use crate::{RpcRequest, RpcResponse};
use libp2p::{
    gossipsub::{
        error::{PublishError, SubscriptionError},
        IdentTopic, MessageId,
    },
    request_response::ResponseChannel,
    PeerId,
};
use std::error::Error as StdError;
use tokio::sync::{mpsc, oneshot};

/// A command for the network to do something. Most likely to send a
/// message of some sort
#[derive(Debug)]
pub enum Command<REQ: RpcRequest, RESP: RpcResponse> {
    /// Instruct the network to provide a list of all peers it is aware of
    PeerList {
        response_sender: oneshot::Sender<Vec<PeerId>>,
    },

    /// Send a gossip message
    PublishGossip {
        /// The topic to publish to
        topic: IdentTopic,

        /// The message to publish
        msg: Vec<u8>,

        response_sender: oneshot::Sender<Result<MessageId, PublishError>>,
    },

    /// Subscribe to a gossip topic
    SubscribeGossip {
        /// The topic to subscribe to
        topic: IdentTopic,
        response_sender: oneshot::Sender<Result<bool, SubscriptionError>>,
    },

    RpcRequest {
        peer: PeerId,
        request: REQ,
        response_sender: oneshot::Sender<Result<RESP, Box<dyn StdError + Send>>>,
    },

    RpcResponse {
        response: RESP,
        channel: ResponseChannel<RESP>,
    },
}

/// Client interface to the p2p network
#[derive(Clone)]
pub struct Client<REQ: RpcRequest, RESP: RpcResponse> {
    sender: mpsc::UnboundedSender<Command<REQ, RESP>>,
}

impl<REQ: RpcRequest, RESP: RpcResponse> Client<REQ, RESP> {
    /// Create a new client.
    pub fn new(sender: mpsc::UnboundedSender<Command<REQ, RESP>>) -> Self {
        Self { sender }
    }

    /// Perform an RPC request to the given peer.
    pub async fn rpc_request(
        &mut self,
        peer: PeerId,
        request: REQ,
    ) -> Result<RESP, Box<dyn StdError + Send>> {
        let (response_sender, response_receiver) = oneshot::channel();
        self.sender
            .send(Command::RpcRequest {
                peer,
                request,
                response_sender,
            })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }

    /// Respond to an incoming RPC request.
    pub async fn rpc_response(&mut self, response: RESP, channel: ResponseChannel<RESP>) {
        self.sender
            .send(Command::RpcResponse { response, channel })
            .expect("Command receiver not to be dropped.");
    }

    /// Subscribe to a gossip topic.
    pub async fn subscribe_gossip(&mut self, topic: IdentTopic) -> Result<bool, SubscriptionError> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender
            .send(Command::SubscribeGossip {
                topic,
                response_sender,
            })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }

    /// Publish a message to a gossip topic.
    pub async fn publish_gossip(
        &mut self,
        topic: IdentTopic,
        msg: Vec<u8>,
    ) -> Result<MessageId, PublishError> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender
            .send(Command::PublishGossip {
                topic,
                msg,
                response_sender,
            })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }

    /// Get list of known peers.
    pub async fn peer_list(&mut self) -> Vec<PeerId> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender
            .send(Command::PeerList { response_sender })
            .expect("Command receiver not to be dropped.");

        response_receiver.await.expect("Sender not be dropped.")
    }
}
