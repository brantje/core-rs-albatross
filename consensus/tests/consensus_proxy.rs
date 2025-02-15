use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::sync::syncer_proxy::SyncerProxy;
use nimiq_consensus::Consensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_network_interface::network::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{fill_micro_blocks_with_txns, UNIT_KEY};
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks, signing_key, voting_key},
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
};
use nimiq_transaction::extended_transaction::ExtTxData;
use nimiq_transaction::{ExecutedTransaction, TransactionFormat};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};
use std::str::FromStr;
use std::sync::Arc;

#[test(tokio::test)]
async fn test_request_transactions_by_address() {
    let mut hub = MockHub::default();

    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(
            VolatileEnvironment::new(11).unwrap(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());
    fill_micro_blocks_with_txns(&producer, &blockchain1, 1, 1);
    // Produce one epoch, such that the transactions are in a finalized epoch
    let num_macro_blocks = (Policy::batches_per_epoch() + 1) as usize;
    produce_macro_blocks(&producer, &blockchain1, num_macro_blocks);

    let net1 = Arc::new(hub.new_network());

    let zkp_prover1 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain1),
        Arc::clone(&net1),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        None,
    )
    .await
    .proxy();

    let blockchain1_proxy = BlockchainProxy::from(&blockchain1);

    let syncer1 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
        net1.subscribe_events(),
    )
    .await;

    let _consensus1 = Consensus::from_network(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        syncer1,
        zkp_prover1.clone(),
    );

    let net2 = Arc::new(hub.new_network());

    let syncer2 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net2),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
        net2.subscribe_events(),
    )
    .await;

    let consensus2 = Consensus::from_network(
        blockchain1_proxy.clone(),
        Arc::clone(&net2),
        syncer2,
        zkp_prover1,
    );

    let consensus_proxy = consensus2.proxy();
    net1.dial_mock(&net2);

    let key_pair = KeyPair::from(PrivateKey::from_str(UNIT_KEY).unwrap());

    let res = consensus_proxy
        .request_transactions_by_address(Address::from(&key_pair.public), 0, vec![], 1, None)
        .await;
    assert!(res.is_ok());

    let txs = res.unwrap();
    assert_eq!(
        txs.iter()
            .filter(|tx| {
                if let ExtTxData::Basic(ExecutedTransaction::Ok(transaction)) = &tx.data {
                    return transaction.format() == TransactionFormat::Basic;
                }
                false
            })
            .count() as u32,
        // There should be one basic transaction in each micro block of the first batch
        Policy::blocks_per_batch() - 1
    );
}
