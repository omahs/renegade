use ring_channel::{ring_channel, RingReceiver, RingSender};
use stats::median;
use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{Arc, RwLock},
    thread,
};

use crate::{
    errors::ReporterError,
    exchanges::{Exchange, ExchangeConnection},
    tokens::Token,
};

fn new_ring_channel<T>() -> (RingSender<T>, RingReceiver<T>) {
    ring_channel::<T>(NonZeroUsize::new(1).unwrap())
}

/// The PriceReport is the universal format for price feeds from all external exchanges.
#[derive(Clone, Copy, Debug, Default)]
pub struct PriceReport {
    /// The midpoint price of the exchange's order book.
    pub midpoint_price: f64,
    /// The time that this update was received by the relayer node.
    pub local_timestamp: u128,
    /// The time that this update was generated by the exchange, if available.
    pub reported_timestamp: Option<u128>,
}

/// The PriceReporter is responsible for opening websocket connection(s) to the specified
/// exchange(s), translating individual PriceReport's into aggregate median PriceReport's, and
/// opening and closing channels for end-consumers to listen to the price feeds.
#[derive(Clone, Debug)]
pub struct PriceReporter {
    base_token: Token,
    quote_token: Token,
    /// Thread-safe HashMap between an Exchange and a vector of senders for PriceReports. As the
    /// PriceReporter processes messages from the various exchanges, this HashMap determines where
    /// price outputs will be sent to.
    price_report_senders: Arc<RwLock<HashMap<Exchange, Vec<RingSender<PriceReport>>>>>,
    /// The latest PriceReport for each Exchange. Used in order to .peek() at each data stream.
    price_report_latest: Arc<RwLock<HashMap<Exchange, PriceReport>>>,
}
impl PriceReporter {
    pub fn new(base_token: Token, quote_token: Token) -> Result<Self, ReporterError> {
        // Use all exchanges.
        let all_exchanges = vec![
            Exchange::Median,
            Exchange::Binance,
            Exchange::Coinbase,
            Exchange::Kraken,
            Exchange::Okx,
            Exchange::UniswapV3,
        ];

        // Create the RingBuffer for the aggregate stream of all updates.
        let (all_price_reports_sender, mut all_price_reports_receiver) =
            new_ring_channel::<(PriceReport, Exchange)>();

        // Connect to all the exchanges, and pipe the price report stream from each connection into
        // the aggregate ring buffer created previously.
        for exchange in all_exchanges.iter().copied() {
            if exchange == Exchange::Median {
                continue;
            }
            let mut price_report_receiver = ExchangeConnection::create_receiver(
                base_token.clone(),
                quote_token.clone(),
                exchange,
            )
            .unwrap();
            let mut all_price_reports_sender_clone = all_price_reports_sender.clone();
            thread::spawn(move || loop {
                let price_report = price_report_receiver.recv().unwrap();
                all_price_reports_sender_clone
                    .send((price_report, exchange))
                    .unwrap();
            });
        }

        // Initialize the thread-safe HashMap's of senders and latests. This first hard-coded
        // RingBuffer will be responsible for streaming all prices and updating the
        // price_report_latest for peeking at each stream.
        let mut price_report_senders_map = HashMap::<Exchange, Vec<RingSender<PriceReport>>>::new();
        let mut price_report_receivers_map_first =
            HashMap::<Exchange, RingReceiver<PriceReport>>::new();
        let mut price_report_latest_map = HashMap::<Exchange, PriceReport>::new();
        for exchange in all_exchanges.iter().copied() {
            let (sender, receiver) = new_ring_channel::<PriceReport>();
            price_report_senders_map.insert(exchange, vec![sender]);
            price_report_receivers_map_first.insert(exchange, receiver);
            price_report_latest_map.insert(exchange, PriceReport::default());
        }
        let price_report_senders = Arc::new(RwLock::new(price_report_senders_map));
        let price_report_latest = Arc::new(RwLock::new(price_report_latest_map));

        // Start worker threads for each exchange that read from the first RingReceiver and update
        // the latest prices.
        for exchange in all_exchanges.iter().copied() {
            let price_report_latest_ref = price_report_latest.clone();
            let mut receiver_first = price_report_receivers_map_first.remove(&exchange).unwrap();
            thread::spawn(move || loop {
                let price_report = receiver_first.recv().unwrap();
                *price_report_latest_ref
                    .write()
                    .unwrap()
                    .get_mut(&exchange)
                    .unwrap() = price_report;
            });
        }

        // Process the aggregate stream and send the aggregate median to each sender in
        // price_report_senders.
        let price_report_senders_ref = price_report_senders.clone();
        let is_named = base_token.is_named() && quote_token.is_named();
        let base_decimals = base_token.get_decimals();
        let quote_decimals = quote_token.get_decimals();
        thread::spawn(move || {
            // This is our internal map from Exchange to most recent PriceReport used for
            // calculating medians. Medians are calculated directly from the most recent valid
            // report from each exchange; we do not do any smoothing over time.
            let mut latest_price_reports = HashMap::<Exchange, PriceReport>::new();
            loop {
                // Receive a new PriceReport from the aggregate stream.
                let (mut price_report_recv, exchange_recv) =
                    all_price_reports_receiver.recv().unwrap();

                // If the received Exchange is UniswapV3 and we are using a Named token pair, then
                // adjust the price in accordance with the decimals.
                if exchange_recv == Exchange::UniswapV3 && is_named {
                    price_report_recv.midpoint_price *= 10_f64.powf(
                        f64::from(base_decimals.unwrap()) - f64::from(quote_decimals.unwrap()),
                    );
                }
                latest_price_reports.insert(exchange_recv, price_report_recv);

                // Before we calculate medians, pass the PriceReport directly through to each
                // RingBuffer sender for the given exchange.
                for sender in price_report_senders_ref
                    .write()
                    .unwrap()
                    .get_mut(&exchange_recv)
                    .unwrap()
                {
                    sender.send(price_report_recv).unwrap();
                }

                // Compute a new median and send it to the median_price_report_sender buffer.
                let median_midpoint_price = median(
                    latest_price_reports
                        .values()
                        .map(|price_report| price_report.midpoint_price),
                )
                .unwrap();
                let median_local_timestamp = median(
                    latest_price_reports
                        .values()
                        .map(|price_report| price_report.local_timestamp),
                )
                .unwrap();
                let median_reported_timestamp = median(
                    latest_price_reports
                        .values()
                        .map(|price_report| price_report.reported_timestamp)
                        .filter(|reported_timestamp| reported_timestamp.is_some())
                        .flatten(),
                )
                .map(|timestamp| timestamp as u128);
                let median_price_report = PriceReport {
                    midpoint_price: median_midpoint_price as f64,
                    local_timestamp: median_local_timestamp as u128,
                    reported_timestamp: median_reported_timestamp,
                };
                for sender in price_report_senders_ref
                    .write()
                    .unwrap()
                    .get_mut(&Exchange::Median)
                    .unwrap()
                {
                    sender.send(median_price_report).unwrap();
                }
            }
        });

        Ok(Self {
            base_token,
            quote_token,
            price_report_senders,
            price_report_latest,
        })
    }

    /// Creates a new ring buffer to report median prices and returns the receiver for consumption
    /// by the callee.
    pub fn create_new_receiver(&self, exchange: Exchange) -> RingReceiver<PriceReport> {
        let (sender, receiver) = new_ring_channel::<PriceReport>();
        (*self.price_report_senders.write().unwrap())
            .get_mut(&exchange)
            .unwrap()
            .push(sender);
        receiver
    }

    /// Returns if this PriceReport is of a "Named" token pair (as opposed to an "Unnamed" pair).
    /// If the PriceReport is Named, then the prices are denominated in USD and largely derived
    /// from centralized exchanges. If the PriceReport is Unnamed, then the prices are derived from
    /// UniswapV3 and do not do fixed-point decimals adjustment.
    pub fn is_named(&self) -> bool {
        self.base_token.is_named() && self.quote_token.is_named()
    }

    /// Nonblocking report of the latest price for a particular exchange.
    pub fn peek(&self, exchange: Exchange) -> Result<PriceReport, ReporterError> {
        Ok(*(*self.price_report_latest.read().unwrap())
            .get(&exchange)
            .unwrap())
    }

    /// Nonblocking report of the latest price for all exchanges.
    pub fn peek_all(&self) -> Result<HashMap<Exchange, PriceReport>, ReporterError> {
        let all_exchanges = vec![
            Exchange::Median,
            Exchange::Binance,
            Exchange::Coinbase,
            Exchange::Kraken,
            Exchange::Okx,
            Exchange::UniswapV3,
        ];
        let mut peek_all = HashMap::<Exchange, PriceReport>::new();
        for exchange in all_exchanges {
            peek_all.insert(exchange, self.peek(exchange)?);
        }
        Ok(peek_all)
    }
}
