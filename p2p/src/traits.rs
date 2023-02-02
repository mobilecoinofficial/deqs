// Copyright (c) 2023 MobileCoin Inc.

// TODO is this used

// use super::{Instruction, Notification};
// use async_trait::async_trait;
// use libp2p::swarm::{ConnectionHandler, IntoConnectionHandler,
// NetworkBehaviour, SwarmEvent}; use tokio::sync::mpsc;

// #[async_trait]
// pub trait EventHandler
// where
//     Self: NetworkBehaviour,
// {
//     async fn handle_event(
//         &mut self,
//         notification_tx: &mpsc::UnboundedSender<Notification>,
//         event: SwarmEvent<
//             Self::OutEvent,
//             <<Self::ConnectionHandler as IntoConnectionHandler>::Handler as
// ConnectionHandler>::Error,         >,
//     );
// }

// #[async_trait]
// pub trait InstructionHandler
// where
//     Self: NetworkBehaviour,
// {
//     async fn handle_instruction(
//         &mut self,
//         notification_tx: &mpsc::UnboundedSender<Notification>,
//         instruction: Instruction,
//     );
// }
