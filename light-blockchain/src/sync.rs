use parking_lot::RwLockUpgradableReadGuard;

use nimiq_block::{Block, BlockError};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, PushError, PushResult,
};
use nimiq_zkp::{verify::verify, NanoProof, ZKP_VERIFYING_KEY};

use crate::blockchain::LightBlockchain;

/// Implements methods to sync a light node.
impl LightBlockchain {
    /// Syncs using a zero-knowledge proof. It receives an election block and a proof that there is
    /// a valid chain between the genesis block and that block.
    /// This brings the node from the genesis block all the way to the most recent election block.
    /// It is the default way to sync for a light node.
    ///
    /// When we get a ZKP from the ZKP component, it is already verified.
    /// We can then set the `trusted_proof` flag to avoid the additional verification.
    pub fn push_zkp(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        proof: NanoProof,
        trusted_proof: bool,
    ) -> Result<PushResult, PushError> {
        // Must be an election block.
        assert!(block.is_election());

        // Checks if the body exists.
        block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        let block_hash = block.hash();

        // Check if we already know this block.
        if this.chain_store.get_chain_info(&block_hash, false).is_ok() {
            return Ok(PushResult::Known);
        }

        if block.block_number() <= this.macro_head.block_number() {
            return Ok(PushResult::Ignored);
        }

        // Perform block intrinsic checks.
        block.verify(false)?;

        // Prepare the inputs to verify the proof.
        let genesis_block_number = this.genesis_block.block_number();
        let genesis_header_hash = <[u8; 32]>::from(this.genesis_block.hash());
        let genesis_pk_tree_root = this
            .genesis_block
            .unwrap_macro_ref()
            .pk_tree_root()
            .ok_or(PushError::InvalidBlock(BlockError::InvalidPkTreeRoot))?;
        let final_block_number = block.block_number();
        let final_header_hash = <[u8; 32]>::from(block_hash.clone());
        let final_pk_tree_root = block
            .unwrap_macro_ref()
            .pk_tree_root()
            .ok_or(PushError::InvalidBlock(BlockError::InvalidPkTreeRoot))?;

        // Verify the zk proof.
        if !trusted_proof {
            let verify_result = verify(
                genesis_block_number,
                genesis_header_hash,
                &genesis_pk_tree_root,
                final_block_number,
                final_header_hash,
                &final_pk_tree_root,
                proof,
                &ZKP_VERIFYING_KEY,
            );

            if verify_result.is_err() || !verify_result.unwrap() {
                return Err(PushError::InvalidZKP);
            }
        }

        // At this point we know that the block is correct. We just have to push it.

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block.clone(), true);

        // Since it's a macro block, we have to clear the ChainStore. If we are syncing for the first
        // time, this should be empty. But we clear it just in case it's not our first time.
        this.chain_store.clear();

        // Store the block chain info.
        this.chain_store.put_chain_info(chain_info);

        // Store the election block header.
        this.chain_store
            .put_election(block.unwrap_macro_ref().header.clone());

        // Update the blockchain.
        this.head = block.clone();

        this.macro_head = block.clone().unwrap_macro();

        this.election_head = block.clone().unwrap_macro();

        this.current_validators = block.validators();

        // We shouldn't log errors if there are no listeners.
        _ = this
            .notifier
            .send(BlockchainEvent::EpochFinalized(block_hash));

        Ok(PushResult::Extended)
    }

    /// Pushes a macro block into the blockchain. This is used when we have already synced to the
    /// most recent election block and now need to push a checkpoint block.
    /// But this function is general enough to allow pushing any macro block (checkpoint or election)
    /// at any state of the node (synced, partially synced, not synced).
    pub fn push_macro(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_macro());

        // Checks if the body exists.
        block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;
        let block_hash = block.hash();

        // Check if we already know this block.
        if this.chain_store.get_chain_info(&block_hash, false).is_ok() {
            return Ok(PushResult::Known);
        }

        if block.block_number() <= this.macro_head.block_number() {
            return Ok(PushResult::Ignored);
        }

        // Perform block intrinsic checks.
        block.verify(false)?;

        // Verify that the block is a valid macro successor to our current macro head.
        block.verify_macro_successor(&this.macro_head)?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(&this.current_validators().unwrap())?;

        // At this point we know that the block is correct. We just have to push it.

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block.clone(), true);

        // Since it's a macro block, we have to clear the ChainStore.
        this.chain_store.clear();

        // Store the block chain info.
        this.chain_store.put_chain_info(chain_info);

        // Update the blockchain.
        this.head = block.clone();

        this.macro_head = block.clone().unwrap_macro();

        // If it's an election block, you have more steps.
        if block.is_election() {
            this.election_head = block.unwrap_macro_ref().clone();

            this.current_validators = block.validators();

            // Store the election block header.
            this.chain_store.put_election(block.unwrap_macro().header);

            // We shouldn't log errors if there are no listeners.
            _ = this
                .notifier
                .send(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            // We shouldn't log errors if there are no listeners.
            _ = this.notifier.send(BlockchainEvent::Finalized(block_hash));
        }

        Ok(PushResult::Extended)
    }
}
