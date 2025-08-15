use anyhow::Result;
use rollup_client::{calculate_signature_hash, create_solana_transaction, RollupClient};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{native_token::LAMPORTS_PER_SOL, signer,signature::Keypair};
use dotenvy::dotenv;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    // let keypair = signer::keypair::read_keypair_file("/home/jvan/.solana/testkey.json").unwrap();
    // let keypair2 = signer::keypair::read_keypair_file("/home/jvan/.solana/mykey_1.json").unwrap();
    let env_keypair = std::env::var("KEYPAIR1")?;
    let env_keypair2 = std::env::var("KEYPAIR2")?;

    let bytes1: Vec<u8> = serde_json::from_str(&env_keypair)?;
    let keypair = Keypair::try_from(&bytes1[..])?;
    
    let bytes2: Vec<u8> = serde_json::from_str(&env_keypair2)?;
    let keypair2 = Keypair::try_from(&bytes2[..])?;

    let rpc_client = RpcClient::new("https://api.devnet.solana.com".into());

    // Get recent blockhash from Solana
    let recent_blockhash = rpc_client.get_latest_blockhash().await.unwrap();

    // Create transaction using the library function
    let tx = create_solana_transaction(&keypair2, &keypair, 1 * LAMPORTS_PER_SOL, recent_blockhash);

    // Create rollup client
    let rollup_client = RollupClient::new("http://127.0.0.1:8080".to_string());

    println!("starting test response...");
    let test_response = rollup_client.health_check().await?;
    println!("{test_response:#?}");

    println!("Submitting transaction...");
    let submit_response = rollup_client.submit_transaction("Me", tx.clone()).await?;
    println!("{submit_response:#?}");
    println!("TX: {:?}", tx);

    let tx_sig = tx.signatures[0].to_string();
    let sig_hash_b58 = calculate_signature_hash(&tx_sig);
    println!("Sig: {}", tx_sig);
    println!("Sig_hash: {:#?}", sig_hash_b58);
    
    
    //comment this while running the client multiple times 
    // println!("Getting transaction...");
    // let tx_resp = rollup_client.get_transaction(&sig_hash_b58).await?;
    // println!("{tx_resp:#?}");

    Ok(())
}
