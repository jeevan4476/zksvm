//! A helper to initialize Solana SVM API's `TransactionBatchProcessor`.

use solana_compute_budget::compute_budget::{
    SVMTransactionExecutionBudget, SVMTransactionExecutionCost,
};
use solana_svm_feature_set::SVMFeatureSet;
use {
    solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1,
    solana_compute_budget::{compute_budget::ComputeBudget,compute_budget_limits::ComputeBudgetLimits},
    solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph, ProgramCacheEntry},
    solana_sdk::{clock::Slot, pubkey::Pubkey, transaction},
    solana_svm::{
        account_loader::CheckedTransactionDetails,
        transaction_processing_callback::TransactionProcessingCallback,
        transaction_processor::TransactionBatchProcessor,
    },
    solana_system_program::system_processor,
    std::collections::HashSet,
    std::sync::{Arc, RwLock},
};
use solana_fee_structure::FeeDetails;

/// In order to use the `TransactionBatchProcessor`, another trait - Solana
/// Program Runtime's `ForkGraph` - must be implemented, to tell the batch
/// processor how to work across forks.
///
/// Since PayTube doesn't use slots or forks, this implementation is mocked.
pub(crate) struct RollupForkGraph {}

impl ForkGraph for RollupForkGraph {
    fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
        BlockRelation::Unknown
    }
}

/// This function encapsulates some initial setup required to tweak the
/// `TransactionBatchProcessor` for use within PayTube.
///
/// We're simply configuring the mocked fork graph on the SVM API's program
/// cache, then adding the System program to the processor's builtins.
pub(crate) fn create_transaction_batch_processor<CB: TransactionProcessingCallback>(
    callbacks: &CB,
    feature_set: &SVMFeatureSet,
    compute_budget: &SVMTransactionExecutionBudget,
    fork_graph: Arc<RwLock<RollupForkGraph>>,
) -> TransactionBatchProcessor<RollupForkGraph> {
    let processor = TransactionBatchProcessor::<RollupForkGraph>::new(
        /* slot */ 1,
        /* epoch */ 1,
        Arc::downgrade(&fork_graph),
        Some(Arc::new(
            create_program_runtime_environment_v1(feature_set, compute_budget, false, false)
                .unwrap(),
        )),
        None,
    );

    // Add the system program builtin.
    processor.add_builtin(
        callbacks,
        solana_system_program::id(),
        "system_program",
        ProgramCacheEntry::new_builtin(
            0,
            b"system_program".len(),
            system_processor::Entrypoint::vm,
        ),
    );

    // Add the BPF Loader v2 builtin, for the SPL Token program.
    processor.add_builtin(
        callbacks,
        solana_sdk_ids::bpf_loader::id(),
        "solana_bpf_loader_program",
        ProgramCacheEntry::new_builtin(
            0,
            b"solana_bpf_loader_program".len(),
            solana_bpf_loader_program::Entrypoint::vm,
        ),
    );

    processor
}

/// This function is also a mock. In the Agave validator, the bank pre-checks
/// transactions before providing them to the SVM API. We mock this step in
/// PayTube, since we don't need to perform such pre-checks.
pub(crate) fn get_transaction_check_results(
    len: usize,
) -> Vec<transaction::Result<CheckedTransactionDetails>> {
    let compute_budget_limit = ComputeBudgetLimits::default();
    vec![
        transaction::Result::Ok(CheckedTransactionDetails::new(
            None,
            Ok(compute_budget_limit.get_compute_budget_and_limits(
                compute_budget_limit.loaded_accounts_bytes,
                FeeDetails::default(),
            )),
        ));
        len
    ]
}
