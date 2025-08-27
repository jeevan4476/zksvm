use anyhow::{anyhow,Result};
use anchor_lang::{InstructionData, ToAccountMetas}; 
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    hash::Hash,
    commitment_config::CommitmentConfig,
    instruction::{AccountMeta,Instruction},
    pubkey::Pubkey,
    signature::Signer,
    signer,
    transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;
use dotenvy::dotenv;
use crossbeam::channel::{Receiver as CBReceiver, Sender as CBSender};
use std::{fs, str::FromStr, time::Duration};
use tokio::time::sleep;
use serde::Deserialize;
use crate::rollupdb::{RollupDBMessage, UpdateProofStatusMessage, ProofStatus, ProofData};

use onchain_verifier::{
    accounts::VerifyGroth16 as VerifyAccounts, instruction::VerifyGroth16Proof as VerifyInstruction,
    Groth16Proof, Groth16VerifyingKey, PublicInputs,
};
use num_bigint::BigUint;

#[derive(Debug, Clone)]
pub struct SettlementJob {
    pub batch_id: String,
    pub proof_data: Option<ProofData>,
    pub transaction_signatures: Vec<String>,
    pub proof_file_path: Option<String>,
}

#[derive(Debug)]
pub enum SettlementResult {
    Success(String), 
    Failed(String),  
    Retry,       
}
//temprary struct to deserialize the vk.json file
#[derive(Clone, Deserialize, Debug)]
pub struct JsonVerifyingKey {
    #[serde(rename = "vk_alpha_1")]
    pub alpha_g1: [String; 3],
    #[serde(rename = "vk_beta_2")]
    pub beta_g2: [[String; 2]; 3],
    #[serde(rename = "vk_gamma_2")]
    pub gamma_g2: [[String; 2]; 3],
    #[serde(rename = "vk_delta_2")]
    pub delta_g2: [[String; 2]; 3],
    #[serde(rename = "IC")]
    pub ic: Vec<[String; 3]>,
}

pub async fn settle_batch_with_proof(
    settlement_job: SettlementJob,
    rollupdb_sender: &CBSender<RollupDBMessage>,
) -> Result<SettlementResult> {
    log::info!("Starting settlement for batch: {}", settlement_job.batch_id);
    
    // here we update proof status to 'posted'
    update_proof_status(
        &settlement_job.batch_id,
        ProofStatus::Posted,
        None,
        rollupdb_sender,
    )?;
    
    match settlement_job.proof_data.clone() {
        Some(proof_data) => {
            settle_with_proof(settlement_job, proof_data, rollupdb_sender).await
        }
        None => {
            log::warn!("No proof data provided for batch: {}, using fallback settlement", settlement_job.batch_id);
            settle_with_fallback_proof(settlement_job, rollupdb_sender).await
        }
    }
}

async fn settle_with_proof(
    settlement_job: SettlementJob,
    proof_data: ProofData,
    rollupdb_sender: &CBSender<RollupDBMessage>,
) -> Result<SettlementResult> {
    log::info!("Attempting proof settlement for batch: {}", settlement_job.batch_id);
    
    dotenv().ok();
    let rpc_client = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".into(),
        CommitmentConfig::confirmed(),
    );
    
    
    let path = std::env::var("KEYPAIR2")?;
    let payer = signer::keypair::read_keypair_file(path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair file: {}", e))?;

    let vk_file  = fs::File::open("build/keys/verification_key_batch.json")?;
    let json_vk:JsonVerifyingKey = serde_json::from_reader(std::io::BufReader::new(vk_file))?;

    let verifying_key = convert_vk_to_onchain_format(&json_vk)?;
    let proof = convert_proof_to_onchain_format(&proof_data)?;
    
    let public_input_file = fs::File::open("build/public_batch.json")?;
    let public_input_str : Vec<String> = serde_json::from_reader(std::io::BufReader::new(public_input_file))?;
    let public_inputs = convert_public_inputs_to_onchain_format(&public_input_str)?;

    let ix = create_onchain_verifier_instruction(&payer.pubkey(), &settlement_job.batch_id, proof, public_inputs, verifying_key)?;

    let recent_blockhash = rpc_client.get_latest_blockhash().await?;
    let transaction = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer],
        recent_blockhash,
    );

     match rpc_client.send_and_confirm_transaction(&transaction).await {
        Ok(signature) => {
            log::info!("Settlement transaction confirmed: {}", signature);
            update_proof_status(&settlement_job.batch_id, ProofStatus::Verified, None, rollupdb_sender)?;
            Ok(SettlementResult::Success(signature.to_string()))
        }
        Err(e) => {
            log::error!(
                "Settlement transaction failed for batch {}: {}",
                settlement_job.batch_id,
                e
            );
            update_proof_status(
                &settlement_job.batch_id,
                ProofStatus::Failed,
                Some(e.to_string()),
                rollupdb_sender,
            )?;
            Ok(SettlementResult::Failed(e.to_string()))
        }
    }
}

// async fn check_valid(settlement_job: SettlementJob,proof_data: &ProofData) -> Result<SettlementResult> {
//     log::info!("Simulating onchain proof verification...");
    
//     sleep(Duration::from_millis(500)).await;
    
//     // here we are checkng if the proof data looks valid
//     let is_valid = !proof_data.pi_a[0].is_empty() 
//                    && !proof_data.pi_b[0][0].is_empty() 
//                    && !proof_data.pi_c[0].is_empty()
//                    && proof_data.protocol == "groth16"
//                    && proof_data.curve == "bn128";
    
//     log::info!("Verification result: {}", if is_valid { "VALID" } else { "INVALID" });
//     Ok(SettlementResult::Failed("invalid".to_string()))
// }



fn create_onchain_verifier_instruction(
    payer: &Pubkey,
    batch_id: &str,
    proof: Groth16Proof,
    public_inputs: PublicInputs,
    verifying_key: Groth16VerifyingKey,
) -> Result<Instruction> {
    let program_id = Pubkey::from_str("Aa3rXCBoxPVZ537nqccEiVsLBoZ2G7gdfNjypM9wP8Yi")?;
    let (proof_account_pda, _) = Pubkey::find_program_address(
        &[b"groth16_proof", payer.as_ref(), batch_id.as_bytes()],
        &program_id,
    );

    let instruction_args = VerifyInstruction {
        proof_id: batch_id.to_string(),
        proof,
        public_inputs,
        verifying_key,
    };

    let accounts = VerifyAccounts {
        authority: *payer,
        proof_account: proof_account_pda,
        system_program: solana_sdk::system_program::id(),
    };

    Ok(Instruction {
        program_id,
        accounts: accounts.to_account_metas(None),
        data: instruction_args.data(),
    })
}

// Settlement for when no proof data is available
async fn settle_with_fallback_proof(
    settlement_job: SettlementJob,
    rollupdb_sender: &CBSender<RollupDBMessage>,
) -> Result<SettlementResult> {
    log::warn!("Using settlement for batch: {}", settlement_job.batch_id);
    
    dotenv().ok();
    let rpc_client = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".into(),
        CommitmentConfig::confirmed(),
    );
    
    let path = std::env::var("KEYPAIR2")?;
    let payer = signer::keypair::read_keypair_file(path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair file: {}", e))?;

    let settle_instruction = system_instruction::transfer(
        &payer.pubkey(),
        &payer.pubkey(),
        0,
    );

    let recent_blockhash = rpc_client.get_latest_blockhash().await?;
    let transaction = Transaction::new_signed_with_payer(
        &[settle_instruction],
        Some(&payer.pubkey()),
        &[&payer],
        recent_blockhash,
    );

    match rpc_client.send_and_confirm_transaction(&transaction).await {
        Ok(signature) => {
            log::info!(" Settlement completed: {}", signature);
            
            // here we update proof status to 'verified'
            update_proof_status(
                &settlement_job.batch_id,
                ProofStatus::Verified,
                Some("Fallback settlement".to_string()),
                rollupdb_sender,
            )?;
            
            Ok(SettlementResult::Success(signature.to_string()))
        }
        Err(e) => {
            log::error!(" Settlement failed: {}", e);
            
            update_proof_status(
                &settlement_job.batch_id,
                ProofStatus::Failed,
                Some(format!("Settlement failed: {}", e)),
                rollupdb_sender,
            )?;
            
            Ok(SettlementResult::Failed(e.to_string()))
        }
    }
}

fn update_proof_status(
    batch_id: &str,
    status: ProofStatus,
    error_message: Option<String>,
    rollupdb_sender: &CBSender<RollupDBMessage>,
) -> Result<()> {
    let update_message = UpdateProofStatusMessage {
        batch_id: batch_id.to_string(),
        new_status: status,
        error_message,
    };
    rollupdb_sender.send(RollupDBMessage {
        update_proof_status: Some(update_message),
        ..Default::default()
    })?;
    
    Ok(())
}

// Settlement worker that processes settlement jobs
pub async fn run_settlement_worker(
    settlement_receiver: CBReceiver<SettlementJob>,
    rollupdb_sender: CBSender<RollupDBMessage>,
) -> Result<()> {
    log::info!("Settlement worker started");
    
    while let Ok(settlement_job) = settlement_receiver.recv() {
        log::info!("Received settlement job for batch: {}", settlement_job.batch_id);
        
        match settle_batch_with_proof(settlement_job.clone(), &rollupdb_sender).await {
            Ok(SettlementResult::Success(signature)) => {
                log::info!(" Settlement successful for batch {}: {}", settlement_job.batch_id, signature);
            }
            Ok(SettlementResult::Failed(error)) => {
                log::error!(" Settlement failed for batch {}: {}", settlement_job.batch_id, error);
            }
            Ok(SettlementResult::Retry) => {
                log::warn!("Settlement needs retry for batch: {}", settlement_job.batch_id);
                // TODO: Retry logic
            }
            Err(e) => {
                log::error!("Settlement error for batch {}: {}", settlement_job.batch_id, e);
            }
        }
    }
    
    Ok(())
}

impl Default for RollupDBMessage {
    fn default() -> Self {
        RollupDBMessage {
            lock_accounts: None,
            add_processed_transaction: None,
            frontend_get_tx: None,
            list_offset: None,
            list_limit: None,
            add_settle_proof: None,
            add_new_data: None,
            store_batch_proof: None,
            update_proof_status: None,
            get_proof_by_batch_id: None,
            get_unsettled_proofs: None,
            retry_failed_proofs: None,
            trigger_retry_cycle:None
        }
    }
}

//helper functions

fn convert_public_inputs_to_onchain_format(inputs: &[String]) -> Result<PublicInputs> {
    let inputs_bytes: Result<Vec<[u8; 32]>> = inputs
        .iter()
        .map(|s| biguint_from_str(s).and_then(biguint_to_32_bytes))
        .collect();

    Ok(PublicInputs {
        inputs: inputs_bytes?,
    })
}

fn convert_proof_to_onchain_format(proof_data: &ProofData) -> Result<Groth16Proof> {
    Ok(Groth16Proof {
        pi_a: g1_from_str_array(&proof_data.pi_a)?,
        pi_b: g2_from_str_array(&proof_data.pi_b)?,
        pi_c: g1_from_str_array(&proof_data.pi_c)?,
    })
}

fn convert_vk_to_onchain_format(json_vk: &JsonVerifyingKey) -> Result<Groth16VerifyingKey> {
    let ic_onchain: Result<Vec<[u8; 64]>> = json_vk.ic.iter().map(g1_from_str_array).collect();
    Ok(Groth16VerifyingKey {
        alpha_g1: g1_from_str_array(&json_vk.alpha_g1)?,
        beta_g2: g2_from_str_array(&json_vk.beta_g2)?,
        gamma_g2: g2_from_str_array(&json_vk.gamma_g2)?,
        delta_g2: g2_from_str_array(&json_vk.delta_g2)?,
        ic: ic_onchain?,
    })
}

fn g1_from_str_array(arr: &[String; 3]) -> Result<[u8; 64]> {
    let mut bytes = [0u8; 64];
    let x = biguint_to_32_bytes(biguint_from_str(&arr[0])?)?;
    let y = biguint_to_32_bytes(biguint_from_str(&arr[1])?)?;
    bytes[..32].copy_from_slice(&x);
    bytes[32..].copy_from_slice(&y);
    Ok(bytes)
}

fn g2_from_str_array(arr: &[[String; 2]; 3]) -> Result<[u8; 128]> {
    let mut bytes = [0u8; 128];
    let x_c1 = biguint_to_32_bytes(biguint_from_str(&arr[0][0])?)?;
    let x_c0 = biguint_to_32_bytes(biguint_from_str(&arr[0][1])?)?;
    let y_c1 = biguint_to_32_bytes(biguint_from_str(&arr[1][0])?)?;
    let y_c0 = biguint_to_32_bytes(biguint_from_str(&arr[1][1])?)?;
    bytes[..32].copy_from_slice(&x_c0);
    bytes[32..64].copy_from_slice(&x_c1);
    bytes[64..96].copy_from_slice(&y_c0);
    bytes[96..].copy_from_slice(&y_c1);
    Ok(bytes)
}

fn biguint_from_str(s: &str) -> Result<BigUint> {
    s.parse::<BigUint>().map_err(|e| anyhow!(e))
}

fn biguint_to_32_bytes(val: BigUint) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    let val_bytes = val.to_bytes_be();
    if val_bytes.len() > 32 {
        return Err(anyhow!("Number too large for 32 bytes"));
    }
    bytes[(32 - val_bytes.len())..].copy_from_slice(&val_bytes);
    Ok(bytes)
}