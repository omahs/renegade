//! Groups handlers for the HTTP API

use async_trait::async_trait;
use circuits::types::{balance::Balance, fee::Fee, order::Order};
use crossbeam::channel::{self, Sender};
use crypto::fields::biguint_to_scalar;
use curve25519_dalek::scalar::Scalar;
use itertools::Itertools;
use std::{
    collections::{HashMap, HashSet},
    iter,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot::channel as oneshot_channel;

use crate::{
    api::http::{
        CreateWalletRequest, GetExchangeHealthStatesRequest, GetExchangeHealthStatesResponse,
        GetReplicasRequest, GetReplicasResponse, OrderBookListRequest, OrderBookListResponse,
        OrderCreateRequest, OrderCreateResponse, PingRequest, PingResponse, WalletAddRequest,
        WalletAddResponse,
    },
    price_reporter::jobs::PriceReporterManagerJob,
    proof_generation::jobs::{ProofJob, ProofManagerJob},
    state::{
        wallet::{PrivateKeyChain, Wallet as StateWallet, WalletIdentifier, WalletMetadata},
        OrderIdentifier, RelayerState,
    },
    MAX_FEES,
};

use super::{error::ApiServerError, routes::TypedHandler, worker::ApiServerConfig};

// ----------------
// | Generic APIs |
// ----------------

/// Handler for the ping route, returns a pong
#[derive(Clone, Debug)]
pub struct PingHandler;
impl PingHandler {
    /// Create a new handler for "/ping"
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl TypedHandler for PingHandler {
    type Request = PingRequest;
    type Response = PingResponse;
    type Error = ApiServerError;

    async fn handle_typed(&self, _req: Self::Request) -> Result<Self::Response, Self::Error> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        Ok(PingResponse { timestamp })
    }
}

// --------------------------
// | Wallet Operations APIs |
// --------------------------

/// Handler for the /wallet/add route
/// TODO: Remove this handler with the endpoint
#[derive(Debug)]
pub struct WalletAddHandler {
    /// A copy of the relayer-global state
    global_state: RelayerState,
}

impl WalletAddHandler {
    /// Create a new handler for the /wallet/add route
    pub fn new(global_state: RelayerState) -> Self {
        Self { global_state }
    }
}

#[async_trait]
impl TypedHandler for WalletAddHandler {
    type Request = WalletAddRequest;
    type Response = WalletAddResponse;
    type Error = ApiServerError;

    async fn handle_typed(&self, req: Self::Request) -> Result<Self::Response, Self::Error> {
        // Create an indexable wallet type from the given wallet
        let wallet_id = WalletIdentifier::new_v4();
        let order_map: HashMap<OrderIdentifier, Order> = req
            .wallet
            .orders
            .iter()
            .map(|order| (OrderIdentifier::new_v4(), order.clone()))
            .collect();

        let balance_map: HashMap<u64, Balance> = req
            .wallet
            .balances
            .iter()
            .map(|balance| (balance.mint, balance.clone()))
            .collect();

        let dummy_secret_keys = PrivateKeyChain {
            sk_root: None,
            sk_match: Scalar::zero(),
            sk_settle: Scalar::zero(),
            sk_view: Scalar::zero(),
        };

        let metadata = WalletMetadata {
            replicas: HashSet::new(),
        };

        let indexable_wallet = StateWallet {
            wallet_id,
            orders: order_map,
            matched_orders: HashMap::new(),
            balances: balance_map,
            fees: req.wallet.fees.clone(),
            public_keys: req.wallet.public_keys.into(),
            secret_keys: dummy_secret_keys,
            randomness: req.wallet.randomness,
            metadata,
        };

        self.global_state.add_wallets(vec![indexable_wallet]);

        Ok(WalletAddResponse { wallet_id })
    }
}

/// Handler for the /wallet/create route
#[derive(Debug)]
pub struct WalletCreateHandler {
    /// The channel to enqueue a proof generation request of `VALID WALLET CREATE` on
    proof_job_queue: Sender<ProofManagerJob>,
}

impl WalletCreateHandler {
    /// Create a new handler for the /wallet/create route
    pub fn new(proof_manager_job_queue: Sender<ProofManagerJob>) -> Self {
        Self {
            proof_job_queue: proof_manager_job_queue,
        }
    }
}

#[async_trait]
impl TypedHandler for WalletCreateHandler {
    type Request = CreateWalletRequest;
    type Response = (); // TODO: Define a response type
    type Error = ApiServerError;

    async fn handle_typed(&self, req: Self::Request) -> Result<Self::Response, Self::Error> {
        // Pad the fees to be of length MAX_FEES
        let fees_padded = req
            .fees
            .into_iter()
            .chain(iter::repeat(Fee::default()))
            .take(MAX_FEES)
            .collect_vec();

        // Forward a request to the proof generation module to build a proof of
        // `VALID WALLET CREATE`
        let (response_sender, response_receiver) = oneshot_channel();
        self.proof_job_queue
            .send(ProofManagerJob {
                type_: ProofJob::ValidWalletCreate {
                    fees: fees_padded,
                    keys: req.keys.into(),
                    randomness: biguint_to_scalar(&req.randomness),
                },
                response_channel: response_sender,
            })
            .map_err(|err| ApiServerError::EnqueueJob(err.to_string()))?;

        // Await a response
        let resp = response_receiver.await.unwrap();
        println!("got proof back: {:?}", resp);
        Ok(())
    }
}

/// Handler for the /wallet/orders/create endpoint
///
/// Adds a new order to the given wallet
#[derive(Clone, Debug)]
pub struct OrderCreateHandler {
    /// A copy of the relayer-global state
    global_state: RelayerState,
}

impl OrderCreateHandler {
    /// Create a new handler for "/wallet/orders/create"
    pub fn new(global_state: RelayerState) -> Self {
        Self { global_state }
    }
}

#[async_trait]
impl TypedHandler for OrderCreateHandler {
    type Request = OrderCreateRequest;
    type Response = OrderCreateResponse;
    type Error = ApiServerError;

    async fn handle_typed(&self, req: Self::Request) -> Result<Self::Response, Self::Error> {
        // Generate an ID for the order
        let order_id = OrderIdentifier::new_v4();
        self.global_state
            .add_order_to_wallet(req.wallet_id, order_id, req.order);

        Ok(OrderCreateResponse { order_id })
    }
}

// -------------------
// | Order Book APIs |
// -------------------

/// Handler for the /orderbook/list endpoint
#[derive(Clone, Debug)]
pub(crate) struct OrderBookListHandler {
    /// A copy of the relayer-global state
    global_state: RelayerState,
}

impl OrderBookListHandler {
    /// Create a new handler for "/orderbook/list"
    pub fn new(global_state: RelayerState) -> Self {
        Self { global_state }
    }
}

#[async_trait]
impl TypedHandler for OrderBookListHandler {
    type Request = OrderBookListRequest;
    type Response = OrderBookListResponse;
    type Error = ApiServerError;

    async fn handle_typed(&self, _req: Self::Request) -> Result<Self::Response, Self::Error> {
        // Fetch all the orders in the book from the global state
        let mut orders = self.global_state.read_order_book().get_all_orders();

        // Do not send validity proofs back with the response to save bandwidth
        for order in orders.iter_mut() {
            order.valid_commit_proof = None;
        }

        Ok(OrderBookListResponse { orders })
    }
}

// ------------------------
// | Price Reporting APIs |
// ------------------------

/// Handler for the /exchange/health_check/ route, returns the health report for each individual
/// exchange and the aggregate median
#[derive(Clone, Debug)]
pub(crate) struct ExchangeHealthStatesHandler {
    /// The config for the API server
    config: ApiServerConfig,
}

impl ExchangeHealthStatesHandler {
    /// Create a new handler for "/exchange/health"
    pub fn new(config: ApiServerConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl TypedHandler for ExchangeHealthStatesHandler {
    type Request = GetExchangeHealthStatesRequest;
    type Response = GetExchangeHealthStatesResponse;
    type Error = ApiServerError;

    async fn handle_typed(&self, req: Self::Request) -> Result<Self::Response, Self::Error> {
        let (price_reporter_state_sender, price_reporter_state_receiver) = channel::unbounded();
        self.config
            .price_reporter_work_queue
            .send(PriceReporterManagerJob::PeekMedian {
                base_token: req.base_token.clone(),
                quote_token: req.quote_token.clone(),
                channel: price_reporter_state_sender,
            })
            .unwrap();
        let (exchange_connection_state_sender, exchange_connection_state_receiver) =
            channel::unbounded();
        self.config
            .price_reporter_work_queue
            .send(PriceReporterManagerJob::PeekAllExchanges {
                base_token: req.base_token,
                quote_token: req.quote_token,
                channel: exchange_connection_state_sender,
            })
            .unwrap();
        Ok(GetExchangeHealthStatesResponse {
            median: price_reporter_state_receiver.recv().unwrap(),
            all_exchanges: exchange_connection_state_receiver.recv().unwrap(),
        })
    }
}

// ---------------------
// | Cluster Info APIs |
// ---------------------

/// Handler for the replicas route, returns the number of replicas a given wallet has
#[derive(Clone, Debug)]
pub struct ReplicasHandler {
    /// The global state of the relayer, used to query information for requests
    global_state: RelayerState,
}

impl ReplicasHandler {
    /// Create a new handler for "/replicas"
    pub fn new(global_state: RelayerState) -> Self {
        Self { global_state }
    }
}

#[async_trait]
impl TypedHandler for ReplicasHandler {
    type Request = GetReplicasRequest;
    type Response = GetReplicasResponse;
    type Error = ApiServerError;

    async fn handle_typed(&self, req: Self::Request) -> Result<Self::Response, Self::Error> {
        let replicas = if let Some(wallet_info) = self
            .global_state
            .read_wallet_index()
            .read_wallet(&req.wallet_id)
        {
            wallet_info.metadata.replicas.clone().into_iter().collect()
        } else {
            vec![]
        };

        Ok(GetReplicasResponse { replicas })
    }
}
