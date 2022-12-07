//! Groups message definitions for cluster management, mostly pubsub

use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

use crate::{
    gossip::types::{ClusterId, WrappedPeerId},
    state::Wallet,
};

/// The topic prefix for the cluster management pubsub topic
///
/// The actual topic name will have the cluster ID postfixed; i.e.
///     cluster-management-{cluster_id}
pub const CLUSTER_MANAGEMENT_TOPIC_PREFIX: &str = "cluster-management";

/// Represents a message containing cluster management information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClusterManagementMessage {
    /// A cluster join message, indicating that a peer wishes to join
    Join(ClusterJoinMessage),
    /// A message indicating that the publisher has replicated the wallets contained
    /// in the message body
    Replicated(ReplicatedMessage),
}

impl From<&ClusterManagementMessage> for Vec<u8> {
    fn from(message: &ClusterManagementMessage) -> Self {
        serde_json::to_vec(&message).unwrap()
    }
}

/// Repesents a pubsub message broadcast when a node joins a cluster
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClusterJoinMessage {
    /// The peer ID of the node joining the cluster
    pub peer_id: WrappedPeerId,
    /// The address that the new peer can be dialed at
    pub addr: Multiaddr,
}

impl From<&ClusterJoinMessage> for Vec<u8> {
    fn from(message: &ClusterJoinMessage) -> Self {
        serde_json::to_vec(&message).unwrap()
    }
}

/// Represents a message indicating a given peer has replicated a set of wallets
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicatedMessage {
    /// The wallets that the peer has newly replicated
    pub wallets: Vec<Wallet>,
    /// The peer that is now replicating the wallet
    pub peer_id: WrappedPeerId,
}

/// A message asking a peer to replicate a set of wallets
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicateRequestBody {
    /// The wallets needing replication
    pub wallets: Vec<Wallet>,
}

/// A message asking a peer to prove they are part of a given cluster
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClusterAuthRequest {
    /// The cluster to prove authorization for
    pub cluster_id: ClusterId,
}

/// A message proving to a requester that the local peer has authorization to
/// join a given cluster.
///
/// A peer proves this by signing their peer ID and the cluster ID with the
/// cluster private key
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClusterAuthResponse {
    /// The body of the response, the signature of which is attached
    pub body: ClusterAuthResponseBody,
    /// The signature of the two former attributes under the cluster private key
    pub signature: Vec<u8>,
}

/// The body of a cluster auth response; signed with the cluster private key
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClusterAuthResponseBody {
    /// The cluster ID
    pub cluster_id: ClusterId,
    /// The ID of the peer proving authorization
    pub peer_id: WrappedPeerId,
}

// Explicit byte serializtaion for hashing and signing
impl From<&ClusterAuthResponseBody> for Vec<u8> {
    fn from(body: &ClusterAuthResponseBody) -> Self {
        serde_json::to_vec(&body).unwrap()
    }
}
