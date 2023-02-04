// Copyright (c) 2023 MobileCoin Inc.

use crate::{RpcRequest, RpcResponse};
use displaydoc::Display;
use libp2p::{
    gossipsub::{
        error::{PublishError, SubscriptionError},
        IdentTopic, MessageId,
    },
    request_response::{OutboundFailure, ResponseChannel},
    Multiaddr, PeerId,
};
use tokio::sync::{mpsc, oneshot};

/// A command for the network to do something. Most likely to send a
/// message of some sort
#[derive(Debug)]
pub enum Command<REQ: RpcRequest, RESP: RpcResponse> {
    /// Instruct the network to provide a list of all peers it is aware of
    PeerList {
        response_sender: oneshot::Sender<Vec<PeerId>>,
    },

    /// Instruct the network to provide a list of addresses it is listening on
    ListenAddrList {
        response_sender: oneshot::Sender<Vec<Multiaddr>>,
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
        response_sender: oneshot::Sender<Result<RESP, Error>>,
    },

    RpcResponse {
        response: RESP,
        channel: ResponseChannel<RESP>,
    },
}

#[derive(Debug, Display)]
pub enum Error {
    /// Failed to send to a channel
    ChannelSend,

    /// Failed to receive from a channel
    ChannelRecv,

    /// Outbound failure: {0}
    OutboundFailure(OutboundFailure),

    /// Gossip subscription: {0}
    GossipSubscription(SubscriptionError),

    /// Gossip publish: {0}
    GossipPublish(PublishError),
}

impl<T> From<mpsc::error::SendError<T>> for Error {
    fn from(_err: mpsc::error::SendError<T>) -> Self {
        Self::ChannelSend
    }
}

impl From<oneshot::error::RecvError> for Error {
    fn from(_err: oneshot::error::RecvError) -> Self {
        Self::ChannelRecv
    }
}

impl From<OutboundFailure> for Error {
    fn from(err: OutboundFailure) -> Self {
        Self::OutboundFailure(err)
    }
}

impl From<SubscriptionError> for Error {
    fn from(err: SubscriptionError) -> Self {
        Self::GossipSubscription(err)
    }
}

impl From<PublishError> for Error {
    fn from(err: PublishError) -> Self {
        Self::GossipPublish(err)
    }
}

impl std::error::Error for Error {}

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
    pub async fn rpc_request(&mut self, peer: PeerId, request: REQ) -> Result<RESP, Error> {
        let (response_sender, response_receiver) = oneshot::channel();
        self.sender.send(Command::RpcRequest {
            peer,
            request,
            response_sender,
        })?;

        response_receiver.await?
    }

    /// Respond to an incoming RPC request.
    pub async fn rpc_response(
        &mut self,
        response: RESP,
        channel: ResponseChannel<RESP>,
    ) -> Result<(), Error> {
        Ok(self
            .sender
            .send(Command::RpcResponse { response, channel })?)
    }

    /// Subscribe to a gossip topic.
    pub async fn subscribe_gossip(&mut self, topic: IdentTopic) -> Result<bool, Error> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender.send(Command::SubscribeGossip {
            topic,
            response_sender,
        })?;

        Ok(response_receiver.await??)
    }

    /// Publish a message to a gossip topic.
    pub async fn publish_gossip(
        &mut self,
        topic: IdentTopic,
        msg: Vec<u8>,
    ) -> Result<MessageId, Error> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender.send(Command::PublishGossip {
            topic,
            msg,
            response_sender,
        })?;

        Ok(response_receiver.await??)
    }

    /// Get list of known peers.
    pub async fn peer_list(&mut self) -> Result<Vec<PeerId>, Error> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender.send(Command::PeerList { response_sender })?;

        Ok(response_receiver.await?)
    }

    //// Get list of listening addresses.
    pub async fn listen_addrs(&mut self) -> Result<Vec<Multiaddr>, Error> {
        let (response_sender, response_receiver) = oneshot::channel();

        self.sender
            .send(Command::ListenAddrList { response_sender })?;

        Ok(response_receiver.await?)
    }
}
