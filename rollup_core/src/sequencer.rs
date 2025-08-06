use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use anyhow::{anyhow, Result};
use async_channel::Sender;
use crossbeam::channel::{Receiver as CBReceiver, Sender as CBSender};
use solana_client::{nonblocking::rpc_client as nonblocking_rpc_client, rpc_client::RpcClient};
use solana_compute_budget::compute_budget::{
    ComputeBudget, SVMTransactionExecutionBudget, SVMTransactionExecutionCost,
};
use solana_program_runtime::{
    invoke_context::{self, EnvironmentConfig, InvokeContext},
    loaded_programs::{
        BlockRelation, ForkGraph, LoadProgramMetrics, ProgramCacheEntry, ProgramCacheForTxBatch,
        ProgramRuntimeEnvironments,
    },
    sysvar_cache,
};
use solana_svm_callback::InvokeContextCallback;
use solana_svm_feature_set::SVMFeatureSet;

use solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1;
use solana_sdk::{
    account::{AccountSharedData, ReadableAccount},
    clock::{Epoch, Slot},
    fee::FeeStructure,
    hash::{hash, Hash},
    pubkey::Pubkey,
    rent::Rent,
    rent_collector::RentCollector,
    transaction::{SanitizedTransaction, Transaction},
    transaction_context::TransactionContext,
};
use solana_svm::{
    transaction_processing_callback::TransactionProcessingCallback,
    transaction_processor::{
        TransactionBatchProcessor, TransactionProcessingConfig, TransactionProcessingEnvironment,
    },
};

use crate::{
    processor::{
        create_transaction_batch_processor, get_transaction_check_results, RollupForkGraph,
    },
    rollupdb::RollupDBMessage,
    settle::settle_state,
};

pub fn run(
    sequencer_receiver_channel: CBReceiver<Transaction>,
    rollupdb_sender: CBSender<RollupDBMessage>,
) -> Result<()> {
    let mut tx_counter = 0u32;

    let rpc_client_temp = RpcClient::new("https://api.devnet.solana.com".to_string());

    println!("Sequencer is running...");
    let rollup_account_loader = RollupAccountLoader::new(&rpc_client_temp);

    while let transaction = sequencer_receiver_channel.recv().unwrap() {
        let accounts_to_lock = transaction.message.account_keys.clone();
        tx_counter += 1;

        println!("send lock accounts to rollupdb");
        // lock accounts in rollupdb to keep paralell execution possible, just like on solana
        rollupdb_sender
            .send(RollupDBMessage {
                lock_accounts: Some(accounts_to_lock),
                frontend_get_tx: None,
                add_settle_proof: None,
                add_processed_transaction: Some(transaction.clone()),
            })
            .map_err(|_| anyhow!("failed to send message to rollupdb"))?;

        // Verify ransaction signatures, integrity

        // Process transaction

        let compute_budget = SVMTransactionExecutionBudget::default();
        let feature_set = SVMFeatureSet::all_enabled();
        let fee_structure = FeeStructure::default();
        let rent_collector = RentCollector::default();
        // let rent_collector = RentCollector::default();

        // Solana runtime.
        let fork_graph = Arc::new(RwLock::new(RollupForkGraph {}));

        println!("Create batch transactions...");
        // // create transaction processor, add accounts and programs, builtins,
        let processor = create_transaction_batch_processor(
            &rollup_account_loader,
            &feature_set,
            &compute_budget,
            Arc::clone(&fork_graph),
        );

        let processing_environment = TransactionProcessingEnvironment {
            blockhash: Hash::default(),
            blockhash_lamports_per_signature: fee_structure.lamports_per_signature,
            epoch_total_stake: 0,
            feature_set,
            rent_collector: Some(&rent_collector),
        };

        let processing_config = TransactionProcessingConfig::default();

        let sanitized = SanitizedTransaction::try_from_legacy_transaction(
            Transaction::from(transaction.clone()),
            &HashSet::new(),
        );

        let sanitized_txs = &[sanitized.unwrap()];

        println!("load and execute...");
        let results = processor.load_and_execute_sanitized_transactions(
            &rollup_account_loader,
            sanitized_txs,
            get_transaction_check_results(1),
            &processing_environment,
            &processing_config,
        );

        // let mut cache = processor.program_cache.write().unwrap();

        // // Initialize the mocked fork graph.
        // // let fork_graph = Arc::new(RwLock::new(PayTubeForkGraph {}));
        // cache.fork_graph = Some(Arc::downgrade(&fork_graph));

        // let rent = Rent::default();

        // let default_env = EnvironmentConfig::new(blockhash, epoch_total_stake, epoch_vote_accounts, feature_set, lamports_per_signature, sysvar_cache)

        // let processing_environment = TransactionProcessingEnvironment {
        //     blockhash: Hash::default(),
        //     epoch_total_stake: None,
        //     epoch_vote_accounts: None,
        //     feature_set: Arc::new(feature_set),
        //     fee_structure: Some(&fee_structure),
        //     lamports_per_signature,
        //     rent_collector: Some(&rent_collector),
        // };

        // Send processed transaction to db for storage and availability
        // println!("send transaction to rollupddb");
        // rollupdb_sender
        //     .send(RollupDBMessage {
        //         lock_accounts: None,
        //         add_processed_transaction: Some(transaction),
        //         frontend_get_tx: None,
        //         add_settle_proof: None,
        //     })
        //     .unwrap();

        // Call settle if transaction amount since last settle hits 10
        if tx_counter >= 2 {
            // TODO Lock db to avoid state changes during settlement

            // TODO Prepare root hash, or your own proof to send to chain

            // Send proof to chain
            println!("SETTLE to L1...");
            let message = b"my rollup state proof or commitment";
            let message_hash = hash(message);

            // ✅ Spawn an async task
            tokio::spawn(async move {
                match settle_state(message_hash.into()).await {
                    Ok(hash) => log::info!("Settle hash: {}", hash),
                    Err(e) => log::error!("Settle failed: {:?}", e),
                }
            });
            tx_counter = 0u32;
        }
    }

    Ok(())
}

pub struct RollupAccountLoader<'a> {
    cache: RwLock<HashMap<Pubkey, AccountSharedData>>,
    rpc_client: &'a RpcClient,
}

impl<'a> RollupAccountLoader<'a> {
    pub fn new(rpc_client: &'a RpcClient) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            rpc_client,
        }
    }
}

impl InvokeContextCallback for RollupAccountLoader<'_> {
    fn get_epoch_stake(&self) -> u64 {
        0
    }

    fn get_epoch_stake_for_vote_account(&self, _vote_address: &Pubkey) -> u64 {
        0
    }

    fn is_precompile(&self, _program_id: &Pubkey) -> bool {
        false
    }

    fn process_precompile(
        &self,
        _program_id: &Pubkey,
        _data: &[u8],
        _instruction_datas: Vec<&[u8]>,
    ) -> std::result::Result<(), solana_sdk::precompiles::PrecompileError> {
        Err(solana_sdk::precompiles::PrecompileError::InvalidPublicKey)
    }
}

// / Implementation of the SVM API's `TransactionProcessingCallback` interface.
// /
// / The SVM API requires this plugin be provided to provide the SVM with the
// / ability to load accounts.
// /
// / In the Agave validator, this implementation is Bank, powered by AccountsDB.
impl TransactionProcessingCallback for RollupAccountLoader<'_> {
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
        if let Some(account) = self.cache.read().unwrap().get(pubkey) {
            return Some(account.clone());
        }

        let account: AccountSharedData = self.rpc_client.get_account(pubkey).ok()?.into();
        self.cache.write().unwrap().insert(*pubkey, account.clone());

        Some(account)
    }

    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.get_account_shared_data(account)
            .and_then(|account| owners.iter().position(|key| account.owner().eq(key)))
    }

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}

    fn inspect_account(
        &self,
        _address: &Pubkey,
        _account_state: solana_svm_callback::AccountState,
        _is_writable: bool,
    ) {
    }

    fn get_current_epoch_vote_account_stake(&self, _vote_address: &Pubkey) -> u64 {
        // Stub implementation: return 0 or implement as needed
        0
    }
}
