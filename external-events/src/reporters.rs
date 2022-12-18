use ring_channel::{ring_channel, RingReceiver, RingSender};
use stats::median;
use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
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
    pub midpoint_price: f32,
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
    pub quote_token: Token,
    pub base_token: Token,
    pub exchanges: Vec<Exchange>,
    /// Thread-safe vector of all senders for the median PriceReport stream. This allows for a
    /// flexible number of data subscribers (by calling PriceReporter::create_new_receiver, which
    /// appends a new sender to this vector).
    median_price_report_senders: Arc<Mutex<Vec<RingSender<PriceReport>>>>,
    /// The latest median PriceReport.
    median_price_report_latest: Arc<Mutex<PriceReport>>,
}
impl PriceReporter {
    /// Given a token pair and exchange identifier, create a new PriceReporter.
    pub fn new(
        quote_token: Token,
        base_token: Token,
        exchanges: Option<Vec<Exchange>>,
    ) -> Result<Self, ReporterError> {
        // If the given exchanges are None, then use all exchanges by default.
        let exchanges = match exchanges {
            Some(exchanges) => exchanges,
            None => vec![
                Exchange::Binance,
                Exchange::Coinbase,
                Exchange::Kraken,
                Exchange::Okx,
            ],
        };

        // Create the RingBuffer for the aggregate stream of all updates.
        let (all_price_reports_sender, mut all_price_reports_receiver) =
            new_ring_channel::<(PriceReport, Exchange)>();

        // Connect to all the exchanges, and pipe the price report stream from each connection into
        // the aggregate ring buffer.
        for exchange in exchanges.to_vec() {
            let mut price_report_receiver =
                ExchangeConnection::new(quote_token, base_token, exchange).unwrap();
            let mut all_price_reports_sender_clone = all_price_reports_sender.clone();
            thread::spawn(move || loop {
                let price_report_recv = price_report_receiver.recv().unwrap();
                all_price_reports_sender_clone
                    .send((price_report_recv, exchange))
                    .unwrap();
            });
        }

        // Create the thread-safe vector of senders and receivers.
        let (sender, mut receiver) = new_ring_channel::<PriceReport>();
        let median_price_report_senders = Arc::new(Mutex::new(vec![sender]));
        let median_price_report_senders_worker = median_price_report_senders.clone();

        // The first hard-coded receiver is responsible for updating the
        // median_price_report_latest.
        let median_price_report_latest = Arc::new(Mutex::new(PriceReport::default()));
        let median_price_report_latest_worker = median_price_report_latest.clone();
        thread::spawn(move || loop {
            let price_report = receiver.recv().unwrap();
            *median_price_report_latest_worker.lock().unwrap() = price_report;
        });

        // Process the aggregate stream and send the aggregate median to each sender in
        // median_price_report_channels.
        thread::spawn(move || {
            let mut latest_price_reports = HashMap::<Exchange, PriceReport>::new();
            loop {
                // Receive a new PriceReport from the aggregate stream.
                let (price_report_recv, exchange_recv) = all_price_reports_receiver.recv().unwrap();
                latest_price_reports.insert(exchange_recv, price_report_recv);

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
                let median_reported_timestamp_unwrapped = median(
                    latest_price_reports
                        .values()
                        .map(|price_report| price_report.reported_timestamp)
                        .filter(|reported_timestamp| reported_timestamp.is_some())
                        .map(|reported_timestamp| reported_timestamp.unwrap()),
                );
                let median_reported_timestamp = match median_reported_timestamp_unwrapped {
                    Some(median_reported_timestamp) => Some(median_reported_timestamp as u128),
                    None => None,
                };
                let median_price_report = PriceReport {
                    midpoint_price: median_midpoint_price as f32,
                    local_timestamp: median_local_timestamp as u128,
                    reported_timestamp: median_reported_timestamp,
                };
                for sender in median_price_report_senders_worker
                    .lock()
                    .unwrap()
                    .iter_mut()
                {
                    sender.send(median_price_report).unwrap();
                }
            }
        });

        Ok(Self {
            quote_token,
            base_token,
            exchanges,
            median_price_report_senders,
            median_price_report_latest,
        })
    }

    /// Creates a new ring buffer to report median prices and returns the receiver for consumption
    /// by the callee.
    pub fn create_new_receiver(&self) -> RingReceiver<PriceReport> {
        let (sender, receiver) = new_ring_channel::<PriceReport>();
        self.median_price_report_senders
            .lock()
            .unwrap()
            .push(sender);
        receiver
    }

    /// Nonblocking report of the latest median price.
    pub fn peek(&self) -> Result<PriceReport, ReporterError> {
        Ok(*self.median_price_report_latest.lock().unwrap())
    }
}
