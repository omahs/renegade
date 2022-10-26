use curve25519_dalek::scalar::Scalar;
use mpc_ristretto::{
    authenticated_scalar::AuthenticatedScalar, beaver::SharedValueSource, network::MpcNetwork,
};

use crate::{
    constants::{MAX_BALANCES, MAX_ORDERS},
    errors::{MpcError, TypeConversionError},
    Allocate,
};

/**
 * Groups types definitions common to the circuit module
 */

// The depth of wallet state trees
pub const WALLET_TREE_DEPTH: usize = 8;

// Represents a wallet and its analog in the constraint system
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wallet {
    pub balances: Vec<Balance>,
    pub orders: Vec<Order>,
    // The maximum number of orders to pad up to when matching, used
    // to shrink the complexity of unit tests
    _max_orders: usize,
    _max_balances: usize,
}

impl Wallet {
    pub fn new(balances: Vec<Balance>, orders: Vec<Order>) -> Self {
        Self::new_with_bounds(balances, orders, MAX_BALANCES, MAX_ORDERS)
    }

    // Allocates a new wallet but allows the caller to specify _max_orders and _max_balances
    // Used in tests to limit the complexity of the match computation
    pub fn new_with_bounds(
        balances: Vec<Balance>,
        orders: Vec<Order>,
        max_balances: usize,
        max_orders: usize,
    ) -> Self {
        Self {
            balances,
            orders,
            _max_balances: max_balances,
            _max_orders: max_orders,
        }
    }

    // Sets the maximum orders that this wallet is padded to when translated
    // into a WalletVar.
    // Used in unit tests to limit the complexity
    pub fn set_max_orders(&mut self, max_orders: usize) {
        assert!(max_orders >= self.orders.len());
        self._max_orders = max_orders;
    }

    pub fn set_max_balances(&mut self, max_balances: usize) {
        assert!(max_balances >= self.balances.len());
        self._max_balances = max_balances;
    }
}

// Represents a balance tuple and its analog in the constraint system
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Balance {
    pub mint: u64,
    pub amount: u64,
}

// Represents an order and its analog in the consraint system
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Order {
    /// The mint (ERC-20 contract address) of the quote token
    pub quote_mint: u64,
    /// The mint (ERC-20 contract address) of the base token
    pub base_mint: u64,
    /// The side this order is for (0 = buy, 1 = sell)
    pub side: OrderSide,
    /// The limit price to be executed at, in units of quote
    pub price: u64,
    /// The amount of base currency to buy or sell
    pub amount: u64,
}

/// Convert a vector of u64s to an Order
impl TryFrom<&[u64]> for Order {
    type Error = TypeConversionError;

    fn try_from(value: &[u64]) -> Result<Self, Self::Error> {
        if value.len() != 5 {
            return Err(TypeConversionError(format!(
                "expected array of length 5, got {:?}",
                value.len()
            )));
        }

        // Check that the side is 0 or 1
        if !(value[2] == 0 || value[2] == 1) {
            return Err(TypeConversionError(format!(
                "Order side must be 0 or 1, got {:?}",
                value[2]
            )));
        }

        Ok(Self {
            quote_mint: value[0],
            base_mint: value[1],
            side: if value[2] == 0 {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            },
            price: value[3],
            amount: value[4],
        })
    }
}

/// Convert an order to a vector of u64s
///
/// Useful for allocating, sharing, serialization, etc
impl From<&Order> for Vec<u64> {
    fn from(o: &Order) -> Self {
        vec![o.quote_mint, o.base_mint, o.side.into(), o.price, o.amount]
    }
}

/// Represents an order that has been allocated in an MPC network
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedOrder<N: MpcNetwork + Send, S: SharedValueSource<Scalar>> {
    /// The mint (ERC-20 contract address) of the quote token
    pub quote_mint: AuthenticatedScalar<N, S>,
    /// The mint (ERC-20 contract address) of the base token
    pub base_mint: AuthenticatedScalar<N, S>,
    /// The side this order is for (0 = buy, 1 = sell)
    pub side: AuthenticatedScalar<N, S>,
    /// The limit price to be executed at, in units of quote
    pub price: AuthenticatedScalar<N, S>,
    /// The amount of base currency to buy or sell
    pub amount: AuthenticatedScalar<N, S>,
}

/// Attempt to parse an authenticated order from a vector of authenticated scalars
impl<N: MpcNetwork + Send, S: SharedValueSource<Scalar>> TryFrom<Vec<AuthenticatedScalar<N, S>>>
    for AuthenticatedOrder<N, S>
{
    type Error = MpcError;

    fn try_from(value: Vec<AuthenticatedScalar<N, S>>) -> Result<Self, Self::Error> {
        if value.len() != 5 {
            return Err(MpcError::SerializationError(format!(
                "Expected 5 elements, got {}",
                value.len()
            )));
        }

        Ok(Self {
            quote_mint: value[0].clone(),
            base_mint: value[1].clone(),
            side: value[2].clone(),
            price: value[3].clone(),
            amount: value[4].clone(),
        })
    }
}

impl<N: MpcNetwork + Send, S: SharedValueSource<Scalar>> Allocate<N, S> for Order {
    type Output = AuthenticatedOrder<N, S>;

    fn allocate(
        &self,
        owning_party: u64,
        fabric: crate::mpc::SharedFabric<N, S>,
    ) -> Result<Self::Output, MpcError> {
        let values_to_allocate: Vec<u64> = self.into();
        let shared_values = fabric
            .borrow_fabric()
            .batch_allocate_private_u64s(owning_party, &values_to_allocate)
            .map_err(|err| MpcError::SharingError(err.to_string()))?;

        Ok(Self::Output::try_from(shared_values).unwrap())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderSide {
    Buy = 0,
    Sell,
}

// Default for an empty order is buy
impl Default for OrderSide {
    fn default() -> Self {
        OrderSide::Buy
    }
}

impl From<OrderSide> for u64 {
    fn from(order_side: OrderSide) -> Self {
        u8::from(order_side) as u64
    }
}

impl From<OrderSide> for u8 {
    fn from(order_side: OrderSide) -> Self {
        match order_side {
            OrderSide::Buy => 0,
            OrderSide::Sell => 1,
        }
    }
}

// The result of a matches operation and its constraint system analog
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MatchResult {
    pub matches1: Vec<Match>,
    pub matches2: Vec<Match>,
}
// Represents a match on a single set of orders overlapping
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SingleMatchResult {
    // Specifies the asset party 1 buys
    pub buy_side1: Match,
    // Specifies the asset party 1 sell
    pub sell_side1: Match,
    // Specifies the asset party 2 buys
    pub buy_side2: Match,
    // Specifies the asset party 2 sells
    pub sell_side2: Match,
}

/// Represents a single match on a set of overlapping orders
/// with values authenticated in an MPC network
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedSingleMatchResult<N: MpcNetwork + Send, S: SharedValueSource<Scalar>> {
    // Specifies the asset party 1 buys
    pub buy_side1: AuthenticatedMatch<N, S>,
    // Specifies the asset party 1 sell
    pub sell_side1: AuthenticatedMatch<N, S>,
    // Specifies the asset party 2 buys
    pub buy_side2: AuthenticatedMatch<N, S>,
    // Specifies the asset party 2 sells
    pub sell_side2: AuthenticatedMatch<N, S>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Match {
    pub mint: u64,
    pub amount: u64,
    pub side: OrderSide,
}

/// Represents a match on one side of the order that is backed by authenticated,
/// network allocated values
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedMatch<N: MpcNetwork + Send, S: SharedValueSource<Scalar>> {
    /// The mint (ERC-20 token) that this match result swaps
    pub mint: AuthenticatedScalar<N, S>,
    /// The amount of the mint token to swap
    pub amount: AuthenticatedScalar<N, S>,
    /// The side (0 is buy, 1 is sell)
    pub side: AuthenticatedScalar<N, S>,
}
