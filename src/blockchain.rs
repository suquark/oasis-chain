//! Oasis blockchain simulator.
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
};

use crate::{
    confidential::ConfidentialCtx, genesis, parity::NullBackend, storage::MemoryMKVS, util,
};
use ekiden_keymanager::client::MockClient;
use ethcore::{
    error::CallError,
    executive::{contract_address, Executed, Executive, TransactOptions},
    filter::Filter,
    log_entry::{LocalizedLogEntry, LogEntry},
    receipt::{LocalizedReceipt, TransactionOutcome},
    state::State,
    transaction::{Action, LocalizedTransaction, SignedTransaction, UnverifiedTransaction},
    types::ids::BlockId,
    vm::{EnvInfo, Error as VmError},
};
use ethereum_types::{Bloom, H256, H64, U256};
use failure::{format_err, Error, Fallible};
use futures::{future, prelude::*, stream};
use hash::{keccak, KECCAK_EMPTY_LIST_RLP};
use lazy_static::lazy_static;
use parity_rpc::v1::types::{
    Block as EthRpcBlock, BlockTransactions as EthRpcBlockTransactions, Header as EthRpcHeader,
    RichBlock as EthRpcRichBlock, RichHeader as EthRpcRichHeader, Transaction as EthRpcTransaction,
};
use tokio_threadpool::{Builder as ThreadPoolBuilder, ThreadPool};

/// Boxed future type.
type BoxFuture<T> = Box<dyn futures::Future<Item = T, Error = failure::Error> + Send>;

/// Block gas limit.
pub const BLOCK_GAS_LIMIT: usize = 16_000_000;
/// Minimum gas price (in gwei).
pub const MIN_GAS_PRICE_GWEI: usize = 1;

/// Simulated blockchain state.
pub struct ChainState {
    mkvs: MemoryMKVS,
    block_number: u64,
    blocks: HashMap<H256, EthereumBlock>,
    block_number_to_hash: HashMap<u64, H256>,
    transactions: HashMap<H256, LocalizedTransaction>,
    receipts: HashMap<H256, LocalizedReceipt>,
}

impl ChainState {
    pub fn new() -> Self {
        // Initialize genesis state.
        let mkvs = MemoryMKVS::new();
        genesis::SPEC
            .ensure_db_good(Box::new(mkvs.clone()), NullBackend, &Default::default())
            .expect("genesis initialization must succeed");

        // Initialize chain state.
        let block_number = 0;
        let mut blocks = HashMap::new();
        let mut block_number_to_hash = HashMap::new();
        let genesis_block = EthereumBlock::new(
            block_number,
            H256::zero(),
            0,
            U256::from(0),
            BLOCK_GAS_LIMIT.into(),
            Default::default(),
        );
        let block_hash = genesis_block.hash();
        blocks.insert(block_hash, genesis_block);
        block_number_to_hash.insert(block_number, block_hash);

        Self {
            mkvs,
            block_number,
            blocks,
            block_number_to_hash,
            transactions: HashMap::new(),
            receipts: HashMap::new(),
        }
    }

    pub fn get_block_by_number(&self, number: u64) -> Option<EthereumBlock> {
        self.block_number_to_hash
            .get(&number)
            .and_then(|hash| self.blocks.get(hash))
            .cloned()
    }
}

/// Simulated blockchain.
pub struct Blockchain {
    gas_price: U256,
    block_gas_limit: U256,
    simulator_pool: Arc<ThreadPool>,
    km_client: Arc<MockClient>,
    chain_state: Arc<RwLock<ChainState>>,
}

impl Blockchain {
    /// Create new simulated blockchain.
    pub fn new(gas_price: U256, block_gas_limit: U256, km_client: Arc<MockClient>) -> Self {
        Self {
            gas_price,
            block_gas_limit,
            simulator_pool: Arc::new(
                ThreadPoolBuilder::new()
                    .name_prefix("simulator-pool-")
                    .build(),
            ),
            km_client,
            chain_state: Arc::new(RwLock::new(ChainState::new())),
        }
    }

    /// Ethereum state snapshot at given block.
    pub fn state(&self, _id: BlockId) -> Fallible<State<NullBackend>> {
        let chain_state = self.chain_state.read().unwrap();

        // TODO: support previous block states
        Ok(State::from_existing(
            Box::new(chain_state.mkvs.clone()),
            NullBackend,
            U256::zero(),       /* account_start_nonce */
            Default::default(), /* factories */
            None,               /* confidential_ctx */
        )?)
    }

    /// Gas price.
    pub fn gas_price(&self) -> U256 {
        self.gas_price
    }

    /// Retrieve an Ethereum block given a block identifier.
    pub fn get_block(
        &self,
        id: BlockId,
    ) -> impl Future<Item = Option<EthereumBlock>, Error = Error> {
        let block: BoxFuture<Option<EthereumBlock>> = match id {
            BlockId::Hash(hash) => Box::new(self.get_block_by_hash(hash)),
            BlockId::Number(number) => Box::new(self.get_block_by_number(number)),
            BlockId::Latest => Box::new(self.get_latest_block().map(|blk| Some(blk))),
            BlockId::Earliest => Box::new(self.get_block_by_number(0)),
        };

        block
    }

    /// The current best block number.
    pub fn best_block_number(&self) -> u64 {
        let chain_state = self.chain_state.read().unwrap();
        chain_state.block_number
    }

    /// Retrieve an Ethereum block given a block identifier.
    ///
    /// If the block is not found it returns an error.
    pub fn get_block_unwrap(
        &self,
        id: BlockId,
    ) -> impl Future<Item = EthereumBlock, Error = Error> {
        self.get_block(id).and_then(|blk| match blk {
            Some(blk) => Ok(blk),
            None => Err(format_err!("block not found")),
        })
    }

    /// Retrieve the latest Ethereum block.
    pub fn get_latest_block(&self) -> impl Future<Item = EthereumBlock, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(
            chain_state
                .get_block_by_number(chain_state.block_number)
                .expect("best block must exist"),
        )
    }

    /// Retrieve a specific Ethereum block, identified by its number.
    pub fn get_block_by_number(
        &self,
        number: u64,
    ) -> impl Future<Item = Option<EthereumBlock>, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(chain_state.get_block_by_number(number))
    }

    /// Retrieve a specific Ethereum block, identified by its block hash.
    pub fn get_block_by_hash(
        &self,
        hash: H256,
    ) -> impl Future<Item = Option<EthereumBlock>, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(chain_state.blocks.get(&hash).cloned())
    }

    /// Retrieve a specific Ethereum transaction, identified by its transaction hash.
    pub fn get_txn_by_hash(
        &self,
        hash: H256,
    ) -> impl Future<Item = Option<LocalizedTransaction>, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(chain_state.transactions.get(&hash).cloned())
    }

    /// Retrieve a specific Ethereum transaction receipt, identified by its transaction
    /// hash.
    pub fn get_txn_receipt_by_hash(
        &self,
        hash: H256,
    ) -> impl Future<Item = Option<LocalizedReceipt>, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(chain_state.receipts.get(&hash).cloned())
    }

    /// Retrieve a specific Ethereum transaction, identified by the block round and
    /// transaction index within the block.
    pub fn get_txn_by_number_and_index(
        &self,
        number: u64,
        index: u32,
    ) -> impl Future<Item = Option<LocalizedTransaction>, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(
            chain_state
                .block_number_to_hash
                .get(&number)
                .and_then(|hash| chain_state.blocks.get(hash))
                .and_then(|blk| blk.transactions.get(index as usize))
                .cloned(),
        )
    }

    /// Retrieve a specific Ethereum transaction, identified by the block hash and
    /// transaction index within the block.
    pub fn get_txn_by_block_hash_and_index(
        &self,
        block_hash: H256,
        index: u32,
    ) -> impl Future<Item = Option<LocalizedTransaction>, Error = Error> {
        let chain_state = self.chain_state.read().unwrap();

        future::ok(
            chain_state
                .blocks
                .get(&block_hash)
                .and_then(|blk| blk.transactions.get(index as usize))
                .cloned(),
        )
    }

    /// Retrieve a specific Ethereum transaction, identified by a block identifier
    /// and transaction index within the block.
    pub fn get_txn(
        &self,
        id: BlockId,
        index: u32,
    ) -> impl Future<Item = Option<LocalizedTransaction>, Error = Error> {
        let txn: BoxFuture<Option<LocalizedTransaction>> = match id {
            BlockId::Hash(hash) => Box::new(self.get_txn_by_block_hash_and_index(hash, index)),
            BlockId::Number(number) => Box::new(self.get_txn_by_number_and_index(number, index)),
            BlockId::Latest => {
                Box::new(self.get_txn_by_number_and_index(self.best_block_number(), index))
            }
            BlockId::Earliest => Box::new(self.get_txn_by_number_and_index(0, index)),
        };

        txn
    }

    /// Submit a raw Ethereum transaction to the chain.
    pub fn send_raw_transaction(
        &self,
        raw: Vec<u8>,
    ) -> impl Future<Item = (H256, ExecutionResult), Error = Error> {
        // Decode transaction.
        let decoded: UnverifiedTransaction = match rlp::decode(&raw) {
            Ok(t) => t,
            Err(_) => return Err(format_err!("Could not decode transaction")).into_future(),
        };

        // Check that gas < block gas limit.
        if decoded.as_unsigned().gas > self.block_gas_limit {
            return Err(format_err!("Requested gas greater than block gas limit")).into_future();
        }

        // Check signature.
        let txn = match SignedTransaction::new(decoded.clone()) {
            Ok(t) => t,
            Err(_) => return Err(format_err!("Invalid signature")).into_future(),
        };

        // Check gas price.
        if txn.gas_price < self.gas_price.into() {
            return Err(format_err!("Insufficient gas price")).into_future();
        }

        // Mine a block with the transaction.
        future::done(self.mine_block(txn))
    }

    /// Mine a block containing the transaction.
    fn mine_block(&self, txn: SignedTransaction) -> Result<(H256, ExecutionResult), Error> {
        let mut chain_state = self.chain_state.write().unwrap();

        // Initialize Ethereum state access functions.
        let best_block = chain_state
            .get_block_by_number(chain_state.block_number)
            .expect("must have a best block");
        let mut state = State::from_existing(
            Box::new(chain_state.mkvs.clone()),
            NullBackend,
            U256::zero(),       /* account_start_nonce */
            Default::default(), /* factories */
            Some(Box::new(ConfidentialCtx::new(
                best_block.hash,
                self.km_client.clone(),
            ))),
        )
        .expect("state initialization must succeed");

        // Initialize Ethereum environment information.
        let number = chain_state.block_number + 1;
        let timestamp = util::get_timestamp();
        let env_info = EnvInfo {
            number,
            author: Default::default(),
            timestamp,
            difficulty: Default::default(),
            gas_limit: self.block_gas_limit,
            // TODO: Get 256 last_hashes.
            last_hashes: Arc::new(vec![best_block.hash]),
            gas_used: Default::default(),
        };

        // Execute the transaction.
        let outcome =
            match state.apply(&env_info, genesis::SPEC.engine.machine(), &txn, false, true) {
                Ok(outcome) => outcome,
                Err(err) => return Err(format_err!("{}", err)),
            };

        // Commit the state updates.
        state.commit().expect("state commit must succeed");

        // Create a block.
        let mut block = EthereumBlock::new(
            number,
            best_block.hash,
            timestamp,
            outcome.receipt.gas_used,
            self.block_gas_limit,
            outcome.receipt.log_bloom,
        );
        let block_hash = block.hash();
        chain_state.block_number = number;

        // Store the txn.
        let txn_hash = txn.hash();
        let localized_txn = LocalizedTransaction {
            signed: txn.clone().into(),
            block_number: number,
            block_hash,
            transaction_index: 0,
            cached_sender: None,
        };
        block.transactions = vec![localized_txn.clone()];
        chain_state.transactions.insert(txn_hash, localized_txn);

        // Store the logs.
        let logs: Vec<LocalizedLogEntry> = outcome
            .receipt
            .logs
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, log)| LocalizedLogEntry {
                entry: log,
                block_hash: block_hash,
                block_number: number,
                transaction_hash: txn_hash,
                transaction_index: 0,
                transaction_log_index: i,
                log_index: i,
            })
            .collect();
        block.logs = logs.clone();

        // Store the receipt.
        let localized_receipt = LocalizedReceipt {
            transaction_hash: txn_hash,
            transaction_index: 0,
            block_hash: block_hash,
            block_number: number,
            cumulative_gas_used: outcome.receipt.gas_used,
            gas_used: outcome.receipt.gas_used,
            contract_address: match txn.action {
                Action::Call(_) => None,
                Action::Create => Some(
                    contract_address(
                        genesis::SPEC.engine.create_address_scheme(number),
                        &txn.sender(),
                        &txn.nonce,
                        &txn.data,
                    )
                    .0,
                ),
            },
            logs: logs,
            log_bloom: outcome.receipt.log_bloom,
            outcome: outcome.receipt.outcome.clone(),
        };
        chain_state.receipts.insert(txn_hash, localized_receipt);

        // Store the block.
        chain_state.blocks.insert(block_hash, block.clone());
        chain_state.block_number_to_hash.insert(number, block_hash);

        // Return the ExecutionResult.
        let result = ExecutionResult {
            cumulative_gas_used: outcome.receipt.gas_used,
            gas_used: outcome.receipt.gas_used,
            log_bloom: outcome.receipt.log_bloom,
            logs: outcome.receipt.logs,
            status_code: match outcome.receipt.outcome {
                TransactionOutcome::StatusCode(code) => code,
                _ => unreachable!("we always use EIP-658 semantics"),
            },
            output: outcome.output.into(),
        };

        info!(
            "Mined block number {:?} containing transaction {:?}. Gas used: {:?}",
            number, txn_hash, result.gas_used
        );

        Ok((txn_hash, result))
    }

    /// Simulate a transaction against a given block.
    ///
    /// The simulated transaction is executed in a dedicated thread pool to
    /// avoid blocking I/O processing.
    ///
    /// # Notes
    ///
    /// Confidential contracts are not supported.
    pub fn simulate_transaction(
        &self,
        transaction: SignedTransaction,
        _id: BlockId,
    ) -> impl Future<Item = Executed, Error = CallError> {
        let simulator_pool = self.simulator_pool.clone();
        let chain_state = self.chain_state.clone();

        // Execute simulation in a dedicated thread pool to avoid blocking
        // I/O processing with simulations.
        simulator_pool.spawn_handle(future::lazy(move || {
            let chain_state = chain_state.read().unwrap();

            let best_block = chain_state
                .get_block_by_number(chain_state.block_number)
                .expect("must have a best block");

            let env_info = EnvInfo {
                number: chain_state.block_number + 1,
                author: Default::default(),
                timestamp: util::get_timestamp(),
                difficulty: Default::default(),
                // TODO: Get 256 last hashes.
                last_hashes: Arc::new(vec![best_block.hash]),
                gas_used: Default::default(),
                gas_limit: U256::max_value(),
            };
            let machine = genesis::SPEC.engine.machine();
            let options = TransactOptions::with_no_tracing()
                .dont_check_nonce()
                .save_output_from_contract();
            let mut state = State::from_existing(
                Box::new(chain_state.mkvs.clone()),
                NullBackend,
                U256::zero(),       /* account_start_nonce */
                Default::default(), /* factories */
                None,               /* confidential_ctx */
            )
            .expect("state initialization must succeed");

            Ok(Executive::new(&mut state, &env_info, machine)
                .transact_virtual(&transaction, options)?)
        }))
    }

    /// Estimates gas against a given block.
    ///
    /// Uses `simulate_transaction` internally.
    ///
    /// # Notes
    ///
    /// Confidential contracts are not supported.
    pub fn estimate_gas(
        &self,
        transaction: SignedTransaction,
        id: BlockId,
    ) -> impl Future<Item = U256, Error = CallError> {
        self.simulate_transaction(transaction, id)
            .inspect(|executed| match &executed.exception {
                Some(VmError::Reverted) | Some(VmError::OutOfGas) => {
                    eprintln!("vm error: {:?}", executed.exception.as_ref().unwrap());
                }
                _ => {}
            })
            .map(|executed| executed.gas_used + executed.refunded)
    }

    /// Looks up logs based on the given filter.
    pub fn logs(
        &self,
        filter: Filter,
    ) -> impl Future<Item = Vec<LocalizedLogEntry>, Error = Error> {
        // Resolve starting and ending blocks.
        let block_numbers = future::join_all(vec![
            Box::new(self.get_block_unwrap(filter.from_block)),
            Box::new(self.get_block_unwrap(filter.to_block)),
        ]);

        // Get blocks.
        let chain_state = self.chain_state.clone();
        let blocks = block_numbers.and_then(move |nums| {
            let from_block = nums[0].number_u64();
            let to_block = nums[1].number_u64();

            stream::iter_ok(from_block..=to_block)
                .map(move |number| {
                    let chain_state = chain_state.read().unwrap();
                    chain_state
                        .get_block_by_number(number)
                        .expect("block should exist")
                })
                .collect()
        });

        // Get logs.
        let logs = blocks
            .map(move |blocks| {
                blocks
                    .into_iter()
                    .flat_map(move |blk| blk.logs.clone())
                    .filter(|log| filter.matches(log))
                    .collect()
            })
            .and_then(|logs: Vec<LocalizedLogEntry>| {
                let mut logs = logs;
                logs.sort_by(|a, b| a.block_number.partial_cmp(&b.block_number).unwrap());
                future::ok(logs)
            });

        Box::new(logs)
    }
}

lazy_static! {
    // Dummy-valued PoW-related block extras.
    static ref BLOCK_EXTRA_INFO: BTreeMap<String, String> = {
        let mut map = BTreeMap::new();
        map.insert("mixHash".into(), format!("0x{:x}", H256::default()));
        map.insert("nonce".into(), format!("0x{:x}", H64::default()));
        map
    };
}

/// Transaction execution result.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecutionResult {
    pub cumulative_gas_used: U256,
    pub gas_used: U256,
    pub log_bloom: Bloom,
    pub logs: Vec<LogEntry>,
    pub status_code: u8,
    #[serde(with = "serde_bytes")]
    pub output: Vec<u8>,
}

/// A wrapper that exposes a simulated Ethereum block.
#[derive(Clone, Debug)]
pub struct EthereumBlock {
    number: u64,
    timestamp: u64,
    hash: H256,
    parent_hash: H256,
    gas_used: U256,
    gas_limit: U256,
    log_bloom: Bloom,
    logs: Vec<LocalizedLogEntry>,
    transactions: Vec<LocalizedTransaction>,
}

impl EthereumBlock {
    /// Create a new Ethereum block.
    pub fn new(
        number: u64,
        parent_hash: H256,
        timestamp: u64,
        gas_used: U256,
        gas_limit: U256,
        log_bloom: Bloom,
    ) -> Self {
        // TODO: better blockhash
        Self {
            number,
            parent_hash,
            timestamp,
            logs: vec![],
            transactions: vec![],
            hash: keccak(number.to_string()).into(),
            gas_used,
            gas_limit,
            log_bloom,
        }
    }

    /// Ethereum block number as an u64.
    pub fn number_u64(&self) -> u64 {
        self.number
    }

    /// Block hash.
    pub fn hash(&self) -> H256 {
        self.hash
    }

    /// Ethereum transactions contained in the block.
    pub fn transactions(&self) -> Vec<LocalizedTransaction> {
        self.transactions.clone()
    }

    /// Retrieve an Ethereum block header with additional metadata.
    pub fn rich_header(&self) -> EthRpcRichHeader {
        EthRpcRichHeader {
            inner: EthRpcHeader {
                hash: Some(self.hash.into()),
                size: None,
                parent_hash: self.parent_hash.into(),
                uncles_hash: KECCAK_EMPTY_LIST_RLP.into(), /* empty list */
                author: Default::default(),
                miner: Default::default(),
                // TODO: state root
                state_root: Default::default(),
                transactions_root: Default::default(),
                receipts_root: Default::default(),
                number: Some(self.number.into()),
                gas_used: self.gas_used.into(),
                gas_limit: self.gas_limit.into(),
                logs_bloom: self.log_bloom.into(),
                timestamp: self.timestamp.into(),
                difficulty: Default::default(),
                seal_fields: vec![],
                extra_data: Default::default(),
            },
            extra_info: { BLOCK_EXTRA_INFO.clone() },
        }
    }

    /// Retrieve an Ethereum block with additional metadata.
    pub fn rich_block(&self, include_txs: bool) -> EthRpcRichBlock {
        let eip86_transition = genesis::SPEC.params().eip86_transition;
        let rich_header = self.rich_header();

        EthRpcRichBlock {
            inner: EthRpcBlock {
                hash: rich_header.hash.clone(),
                size: rich_header.size,
                parent_hash: rich_header.parent_hash.clone(),
                uncles_hash: rich_header.uncles_hash.clone(),
                author: rich_header.author.clone(),
                miner: rich_header.miner.clone(),
                state_root: rich_header.state_root.clone(),
                transactions_root: rich_header.transactions_root.clone(),
                receipts_root: rich_header.receipts_root.clone(),
                number: rich_header.number,
                gas_used: rich_header.gas_used,
                gas_limit: rich_header.gas_limit,
                logs_bloom: Some(rich_header.logs_bloom.clone()),
                timestamp: rich_header.timestamp,
                difficulty: rich_header.difficulty,
                total_difficulty: None,
                seal_fields: vec![],
                uncles: vec![],
                transactions: match include_txs {
                    true => EthRpcBlockTransactions::Full(
                        self.transactions
                            .clone()
                            .into_iter()
                            .map(|txn| EthRpcTransaction::from_localized(txn, eip86_transition))
                            .collect(),
                    ),
                    false => EthRpcBlockTransactions::Hashes(
                        self.transactions
                            .clone()
                            .into_iter()
                            .map(|txn| txn.signed.hash().into())
                            .collect(),
                    ),
                },
                extra_data: Default::default(),
            },
            extra_info: rich_header.extra_info.clone(),
        }
    }
}
