use async_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use solana_sdk::{
    account::AccountSharedData, keccak::Hash, pubkey::Pubkey, transaction::Transaction,
};

use crossbeam::channel::{Receiver as CBReceiver, Sender as CBSender};
use std::{
    collections::{HashMap, HashSet},
    default,
};

use crate::frontend::FrontendMessage;

pub struct RollupDBMessage {
    pub lock_accounts: Option<Vec<Pubkey>>,
    pub add_processed_transaction: Option<Transaction>,
    pub frontend_get_tx: Option<Hash>,
    pub add_settle_proof: Option<String>,
}

#[derive(Debug, Default)]
pub struct RollupDB {
    accounts_db: HashMap<Pubkey, AccountSharedData>,
    locked_accounts: HashMap<Pubkey, AccountSharedData>,
    transactions: HashMap<Hash, Transaction>,
}

impl RollupDB {
    pub async fn run(
        rollup_db_receiver: CBReceiver<RollupDBMessage>,
        frontend_sender: Sender<FrontendMessage>,
    ) {
        let mut db = RollupDB {
            accounts_db: HashMap::new(),
            locked_accounts: HashMap::new(),
            transactions: HashMap::new(),
        };

        while let Ok(message) = rollup_db_receiver.recv() {
            log::info!("got MMESSAGE");
            log::info!("GETTX: {:?}", message.frontend_get_tx);
            log::info!("LOCKOUTS: {:?}", message.lock_accounts);
            log::info!("ADDPROCESSEDTX: {:?}", message.add_processed_transaction);

            // ✅ verwerk lock_accounts indien aanwezig
            if let Some(accounts_to_lock) = &message.lock_accounts {
                log::info!("got LOCKUP ACCOUNTS");
                for pubkey in accounts_to_lock {
                    if let Some(account) = db.accounts_db.remove(pubkey) {
                        db.locked_accounts.insert(*pubkey, account);
                    }
                }
            }

            // ✅ verwerk add_processed_transaction indien aanwezig
            if let Some(tx) = &message.add_processed_transaction {
                log::info!("ADD ROLLUPDB tx");
                let hash = tx.signatures[0];
                log::info!("signature: {}", hash);
                let tx_hash = solana_sdk::keccak::hashv(&[hash.as_ref()]);
                log::info!("txhash: {}", tx_hash);
                db.transactions.insert(tx_hash, tx.clone());
                log::info!("INSERTED TX IN DB");
            }

            // ✅ verwerk frontend_get_tx indien aanwezig
            if let Some(get_this_hash_tx) = &message.frontend_get_tx {
                log::info!("got get tx hash api");

                match db.transactions.get(get_this_hash_tx) {
                    Some(req_tx) => {
                        frontend_sender
                            .send(FrontendMessage {
                                transaction: Some(req_tx.clone()),
                                get_tx: None,
                            })
                            .await
                            .unwrap();
                    }
                    None => {
                        log::warn!("No transaction found for hash: {}", get_this_hash_tx);
                    }
                }
            }
        }
    }
}

