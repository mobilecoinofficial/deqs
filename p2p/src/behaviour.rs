// Copyright (c) 2023 MobileCoin Inc.

use super::Error;
use libp2p::{
    gossipsub::{
        Gossipsub, GossipsubEvent, GossipsubMessage, MessageAuthenticity, MessageId, ValidationMode,
    },
    identify, identity,
    kad::{record::store::MemoryStore, Kademlia, KademliaConfig, KademliaEvent},
    ping, NetworkBehaviour, PeerId,
};
use libp2p_swarm::keep_alive;
use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    iter,
};
use tokio::time::Duration;
use void::Void;

const KADEMLIA_PROTO_NAME: &[u8] = b"/mc/deqs/kad/1.0.0";
const IDENTIFY_PROTO_NAME: &str = "/mc/deqs/ide/1.0.0";
const GOSSIPSUB_PROTO_ID_PREFIX: &str = "mc/deqs/gossipsub";

/// Custom behaviour for the p2p network (this is a collection of protocols
/// bundled together into a single behaviour)
#[derive(NetworkBehaviour)]
#[behaviour(out_event = "OutEvent")]
pub struct Behaviour {
    pub keep_alive: keep_alive::Behaviour,
    //ping: ping::Behaviour,
    /// - `identify::Behaviour`: when peers connect and they both support this
    ///   protocol, they exchange `IdentityInfo`. We then uses this info to add
    ///   their listen addresses to the Kademlia DHT. Without
    ///   `identify::Behaviour`, the DHT would propagate the peer ids of peers,
    ///   but not their listen addresses, making it impossible for peers to
    ///   connect to them.
    pub identify: identify::Behaviour,

    // - `Kademlia`: this is the Distributed Hash Table implementation,
    ///   primarily used to distribute info about other peers on the
    ///   network this peer knows about to connected peers. This is a core
    ///   feature of `Kademlia` that triggers automatically.
    ///   This enables so-called DHT-routing, which enables peers to send
    ///   messages to peers they are not directly connected to.
    pub kademlia: Kademlia<MemoryStore>,

    /// - `Gossipsub`: this takes care of sending pubsub messages to all peers
    ///   this peer is aware of on a certain topic. Subscribe/unsubscribe
    ///   messages are also propagated. This works well in combination with
    ///   `identify::Behaviour` and `Kademlia`, because they ensure that
    ///   Gossipsub messages not only reach directly connected peers, but all
    ///   peers that can be reached through the DHT routing.
    pub gossipsub: Gossipsub,
}

impl Behaviour {
    pub fn new(local_key: &identity::Keypair) -> Result<Self, Error> {
        let gossipsub = Self::create_gossipsub(local_key)?;
        let kademlia = Self::create_kademlia(PeerId::from(local_key.public()));
        let identify = Self::create_identify(local_key);

        Ok(Self {
            keep_alive: keep_alive::Behaviour::default(),
            //ping: ping::Behaviour::default(),
            kademlia,
            gossipsub,
            identify,
        })
    }

    fn create_gossipsub(local_key: &identity::Keypair) -> Result<Gossipsub, Error> {
        // To content-address message, we can take the hash of message and use it as an
        // ID.
        let message_id_fn = |message: &GossipsubMessage| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            MessageId::from(s.finish().to_string())
        };

        let gossipsub_config = libp2p::gossipsub::GossipsubConfigBuilder::default()
            .protocol_id_prefix(GOSSIPSUB_PROTO_ID_PREFIX)
            // This is set to aid debugging by not cluttering the log space
            .heartbeat_interval(Duration::from_secs(10))
            // This sets the kind of message validation. The default is Strict (enforce message
            // signing)
            .validation_mode(ValidationMode::Strict)
            // content-address messages. No two messages of the same content will be propagated.
            .message_id_fn(message_id_fn)
            .build()
            .map_err(Error::GossipsubBuild)?;

        Gossipsub::new(
            MessageAuthenticity::Signed(local_key.clone()),
            gossipsub_config,
        )
        .map_err(Error::GossipsubNew)
    }

    fn create_kademlia(local_peer_id: PeerId) -> Kademlia<MemoryStore> {
        let mut cfg = KademliaConfig::default();
        cfg.set_provider_publication_interval(Some(Duration::from_secs(10)));
        cfg.set_query_timeout(Duration::from_secs(5 * 60));
        cfg.set_protocol_names(iter::once(Cow::Borrowed(KADEMLIA_PROTO_NAME)).collect());
        //cfg.set_provider_publication_interval(Some(Duration::from_secs(1)));

        let store = MemoryStore::new(local_peer_id);
        Kademlia::with_config(local_peer_id, store, cfg)
    }

    fn create_identify(local_key: &identity::Keypair) -> identify::Behaviour {
        identify::Behaviour::new(identify::Config::new(
            IDENTIFY_PROTO_NAME.to_string(),
            local_key.public(),
        ))
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum OutEvent {
    Ping(ping::Event),
    Void(Void),
    Kademlia(KademliaEvent),
    Identify(identify::Event),
    Gossipsub(GossipsubEvent),
}
impl From<ping::Event> for OutEvent {
    fn from(event: ping::Event) -> Self {
        OutEvent::Ping(event)
    }
}
impl From<Void> for OutEvent {
    fn from(event: Void) -> Self {
        OutEvent::Void(event)
    }
}
impl From<KademliaEvent> for OutEvent {
    fn from(event: KademliaEvent) -> Self {
        OutEvent::Kademlia(event)
    }
}
impl From<identify::Event> for OutEvent {
    fn from(event: identify::Event) -> Self {
        OutEvent::Identify(event)
    }
}
impl From<GossipsubEvent> for OutEvent {
    fn from(event: GossipsubEvent) -> Self {
        OutEvent::Gossipsub(event)
    }
}
