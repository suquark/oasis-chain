// Copyright 2015-2018 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

//! Eth Filter RPC implementation

use std::sync::Arc;

use ethcore::{filter::Filter as EthcoreFilter, ids::BlockId};
use failure::format_err;
use jsonrpc_core::{
    futures::{future, prelude::*, stream},
    BoxFuture, Result,
};
use parity_rpc::v1::{
    helpers::{errors, limit_logs, PollFilter, PollManager},
    traits::EthFilter,
    types::{Filter, FilterChanges, Index, Log, H256 as RpcH256, U256 as RpcU256},
};
use parking_lot::Mutex;

use crate::{blockchain::Blockchain, util::jsonrpc_error};

/// Eth filter rpc implementation for a full node.
pub struct EthFilterClient {
    blockchain: Arc<Blockchain>,
    polls: Arc<Mutex<PollManager<PollFilter>>>,
}

impl EthFilterClient {
    /// Creates new Eth filter client.
    pub fn new(blockchain: Arc<Blockchain>) -> Self {
        EthFilterClient {
            blockchain,
            polls: Arc::new(Mutex::new(PollManager::new())),
        }
    }
}

impl EthFilter for EthFilterClient {
    fn new_filter(&self, filter: Filter) -> BoxFuture<RpcU256> {
        let polls = self.polls.clone();
        Box::new(
            self.blockchain
                .get_latest_block()
                .map_err(jsonrpc_error)
                .map(move |blk| {
                    let mut polls = polls.lock();
                    let id = polls.create_poll(PollFilter::Logs(
                        blk.number_u64(),
                        Default::default(),
                        filter,
                    ));

                    id.into()
                }),
        )
    }

    fn new_block_filter(&self) -> BoxFuture<RpcU256> {
        let polls = self.polls.clone();
        Box::new(
            self.blockchain
                .get_latest_block()
                .map_err(jsonrpc_error)
                .map(move |blk| {
                    let mut polls = polls.lock();
                    // +1, since we don't want to include the current block.
                    let id = polls.create_poll(PollFilter::Block(blk.number_u64() + 1));

                    id.into()
                }),
        )
    }

    fn new_pending_transaction_filter(&self) -> Result<RpcU256> {
        // We don't have pending transactions, so this is a no-op filter.
        let mut polls = self.polls.lock();
        let id = polls.create_poll(PollFilter::PendingTransaction(vec![]));
        Ok(id.into())
    }

    fn filter_changes(&self, index: Index) -> BoxFuture<FilterChanges> {
        let polls = self.polls.clone();
        let blockchain = self.blockchain.clone();

        Box::new(
            self.blockchain
                .get_latest_block()
                .map_err(jsonrpc_error)
                .and_then(move |blk| -> BoxFuture<FilterChanges> {
                    let mut polls = polls.lock();
                    match polls.poll_mut(&index.value()) {
                        None => Box::new(future::err(errors::filter_not_found())),
                        Some(PollFilter::Block(ref mut number)) => {
                            // TODO: Should we support block range fetch?
                            let updates = Box::new(
                                stream::iter_ok(*number..=blk.number_u64())
                                    .and_then(move |number| blockchain.get_block_by_number(number))
                                    .and_then(|blk| match blk {
                                        Some(blk) => Ok(blk),
                                        None => Err(format_err!("block not found")),
                                    })
                                    .map(|blk| RpcH256::from(blk.hash()))
                                    .collect()
                                    .map_err(jsonrpc_error)
                                    .map(|hashes| FilterChanges::Hashes(hashes)),
                            );

                            *number = blk.number_u64();
                            updates
                        }
                        Some(PollFilter::PendingTransaction(_)) => {
                            // We don't have pending transactions, so this is a no-op filter.
                            Box::new(future::ok(FilterChanges::Hashes(vec![])))
                        }
                        Some(PollFilter::Logs(ref mut block_number, _, ref filter)) => {
                            // Build appropriate filter.
                            let mut filter: EthcoreFilter = filter.clone().into();
                            filter.from_block = BlockId::Number(*block_number);
                            filter.to_block = BlockId::Latest;

                            // Save the number of the next block as a first block from which
                            // we want to get logs.
                            *block_number = blk.number_u64() + 1;

                            let limit = filter.limit;
                            Box::new(
                                blockchain
                                    .clone()
                                    .logs(filter)
                                    .map_err(jsonrpc_error)
                                    .map(|logs| logs.into_iter().map(Into::into).collect())
                                    .map(move |logs| limit_logs(logs, limit))
                                    .map(FilterChanges::Logs),
                            )
                        }
                    }
                }),
        )
    }

    fn filter_logs(&self, index: Index) -> BoxFuture<Vec<Log>> {
        let filter = {
            let mut polls = self.polls.lock();

            match polls.poll(&index.value()) {
                Some(&PollFilter::Logs(.., ref filter)) => filter.clone(),
                Some(_) => return Box::new(future::ok(Vec::new())),
                None => return Box::new(future::err(errors::filter_not_found())),
            }
        };

        let filter: EthcoreFilter = filter.into();
        let limit = filter.limit;

        Box::new(
            self.blockchain
                .clone()
                .logs(filter)
                .map_err(jsonrpc_error)
                .map(|logs| logs.into_iter().map(Into::into).collect())
                .map(move |logs| limit_logs(logs, limit)),
        )
    }

    fn uninstall_filter(&self, index: Index) -> Result<bool> {
        Ok(self.polls.lock().remove_poll(&index.value()))
    }
}
