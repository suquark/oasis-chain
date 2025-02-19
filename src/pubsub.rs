//! Pub/sub support.
use std::{
    process::abort,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock, Weak,
    },
    time::Duration,
};

use ethcore::filter::TxEntry;
use futures::prelude::*;
use log::error;
use tokio::timer::Interval;

use crate::blockchain::Blockchain;

/// An actor listening to chain events.
///
/// All notifications are delivered in a future task context.
pub trait Listener: Send + Sync {
    fn notify_blocks(&self, from_block: u64, to_block: u64);

    fn notify_completed_transaction(&self, entry: &TxEntry, output: Vec<u8>);
}

struct Inner {
    blockchain: Arc<Blockchain>,
    last_notified_block: AtomicU64,
    listeners: RwLock<Vec<Weak<dyn Listener>>>,
}

pub struct Broker {
    inner: Arc<Inner>,
}

impl Broker {
    pub fn new(blockchain: Arc<Blockchain>) -> Self {
        Self {
            inner: Arc::new(Inner {
                blockchain,
                last_notified_block: AtomicU64::new(0),
                listeners: RwLock::new(vec![]),
            }),
        }
    }

    pub fn add_listener(&self, listener: Weak<dyn Listener>) {
        let mut listeners = self.inner.listeners.write().unwrap();
        listeners.push(listener);
    }

    pub fn start(&self, interval: Duration) -> impl Future<Item = (), Error = ()> {
        let inner = self.inner.clone();

        Interval::new_interval(interval)
            .map_err(Into::into)
            .for_each(move |_| {
                // Get latest block and notify all listeners of the difference.
                let inner = inner.clone();
                inner.blockchain.get_latest_block().map(move |blk| {
                    let last_notified_block = inner.last_notified_block.load(Ordering::SeqCst);
                    let listeners = inner.listeners.read().unwrap();

                    let to = blk.number_u64();

                    // If there are no new blocks, return early.
                    if to <= last_notified_block {
                        return;
                    }

                    let from = last_notified_block + 1;

                    for listener in listeners.iter() {
                        if let Some(listener) = listener.upgrade() {
                            listener.notify_blocks(from, to);
                        }
                    }

                    inner.last_notified_block.store(to, Ordering::SeqCst);
                })
            })
            .map_err(move |err| {
                error!("Pub/sub notifier error: {:?}", err,);
                abort();
            })
    }
}
