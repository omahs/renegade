//! Groups type definitions relevant to all modules and at the top level

use circuits::zk_circuits::valid_commitments::ValidCommitments;
use serde::{Deserialize, Serialize};

use crate::{
    price_reporter::reporter::PriceReport,
    state::{
        wallet::{Wallet, WalletIdentifier},
        NetworkOrderState, OrderIdentifier,
    },
    MAX_BALANCES, MAX_FEES, MAX_ORDERS,
};

// ----------------------------------
// | Circuit Default Generics Types |
// ----------------------------------

/// `VALID COMMITMENTS` with default state element sizing
pub type SizedValidCommitments = ValidCommitments<MAX_BALANCES, MAX_ORDERS, MAX_FEES>;

// ----------------------
// | Pubsub Topic Names |
// ----------------------

/// The topic published to when the handshake manager begins a new
/// match computation with a peer
pub const HANDSHAKE_STATUS_TOPIC: &str = "handshakes";
/// The topic published to when a state change occurs on an order
pub const ORDER_STATE_CHANGE_TOPIC: &str = "order-state";
/// The topic published to when a wallet is updated
pub const WALLET_UPDATE_TOPIC: &str = "wallet-update";

// ----------------------------
// | System Bus Message Types |
// ----------------------------

/// A message type for generic system bus messages, broadcast to all modules
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum SystemBusMessage {
    /// A message indicating that a handshake with a peer has started
    HandshakeInProgress {
        /// The order_id of the local party
        local_order_id: OrderIdentifier,
        /// The order_id of the remote peer
        peer_order_id: OrderIdentifier,
    },
    /// A message indicating that a handshake with a peer has completed
    HandshakeCompleted {
        /// The order_id of the local party
        local_order_id: OrderIdentifier,
        /// The order_id of the remote peer
        peer_order_id: OrderIdentifier,
    },
    /// A message indicating that an order has changed state in the local order
    /// book
    OrderStateChange {
        /// The order identifier
        order_id: OrderIdentifier,
        /// The old state of the order
        prev_state: NetworkOrderState,
        /// The new state of the order
        new_state: NetworkOrderState,
    },
    /// A message indicating that a new median PriceReport has been published
    PriceReportMedian(PriceReport),
    /// A message indicating that a new individual exchange PriceReport has been published
    PriceReportExchange(PriceReport),
    /// A message indicating that a wallet has been updated, possibly by a match
    WalletUpdate {
        /// The ID of the wallet that was updated
        wallet_id: WalletIdentifier,
        /// The updated wallet
        wallet: Wallet,
    },
}

/// A wrapper around a SystemBusMessage containing the topic, used for serializing websocket
/// messages to clients
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemBusMessageWithTopic {
    /// The topic of this message
    pub topic: String,
    /// The event itself
    pub event: SystemBusMessage,
}
