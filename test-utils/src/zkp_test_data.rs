use std::{fs::File, path::Path, sync::Arc};

use ark_ff::ToConstraintField;
use ark_groth16::VerifyingKey;
use ark_mnt6_753::MNT6_753;
use ark_serialize::CanonicalDeserialize;
use nimiq_zkp_component::types::ZKProof;
use parking_lot::RwLock;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_genesis::NetworkInfo;
use nimiq_zkp_circuits::test_setup::ToxicWaste;
use nimiq_zkp_primitives::{state_commitment, vk_commitment};

pub fn get_base_seed() -> ChaCha20Rng {
    let seed = [
        1, 0, 52, 0, 0, 0, 0, 0, 1, 0, 10, 0, 22, 32, 0, 0, 2, 0, 55, 49, 0, 11, 0, 0, 3, 0, 0, 0,
        0, 0, 2, 92,
    ];
    ChaCha20Rng::from_seed(seed)
}

pub const ZKP_TEST_BIN_NAME: &str = "nimiq-test-prove";
/// The path to the zkp directory for tests relative to the test binaries.
/// This should be used while running unit tests.
pub const ZKP_TEST_KEYS_PATH: &str = "../.zkp_tests";
/// The path to the zkp directory for tests relative to the project root.
/// This should be used while running test related binaries.
/// We have copies of this constant in several places that need to be updated when updating this.
pub const DEFAULT_TEST_KEYS_PATH: &str = ".zkp_tests";

pub fn zkp_test_exe() -> std::path::PathBuf {
    // Cargo puts the integration test binary in target/debug/deps
    let current_exe =
        std::env::current_exe().expect("Failed to get the path of the integration test binary");
    let current_dir = current_exe
        .parent()
        .expect("Failed to get the directory of the integration test binary");

    let test_bin_dir = current_dir
        .parent()
        .expect("Failed to get the binary folder");
    let mut path = test_bin_dir.to_owned();

    path.push(ZKP_TEST_BIN_NAME);
    path.set_extension(std::env::consts::EXE_EXTENSION);

    assert!(
        path.exists(),
        "Run `cargo test --all-features` to build the test prover binary at {path:?}"
    );
    path
}

pub fn load_merger_wrapper_simulator(path: &Path) -> Option<ToxicWaste<MNT6_753>> {
    let file = File::open(path.join("toxic_waste.bin")).ok()?;
    ToxicWaste::deserialize_uncompressed(file).ok()
}

/// This function simulates a proof for the Merger Wrapper circuit, which implicitly is a proof for
/// the entire light macro sync. It is very fast, shouldn't take more than a second, even on older
/// computers.
pub fn simulate_merger_wrapper(
    path: &Path,
    blockchain: &Arc<RwLock<Blockchain>>,
    verifying_key: &VerifyingKey<MNT6_753>,
    rng: &mut impl Rng,
) -> ZKProof {
    let block = blockchain.read().state.election_head.clone();
    let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
    let genesis_block = network_info.genesis_block().unwrap_macro();

    let genesis_block_number = genesis_block.block_number();
    let genesis_header_hash = genesis_block.hash().into();
    let genesis_pk_tree_root = genesis_block
        .pk_tree_root()
        .expect("Missing pk tree root in macro block.");

    let final_block_number = block.block_number();
    let final_header_hash = block.hash().into();
    let final_pk_tree_root = block
        .pk_tree_root()
        .expect("Missing pk tree root in macro block.");

    // Prepare the inputs.
    let mut inputs = vec![];

    inputs.append(
        &mut state_commitment(
            genesis_block_number,
            &genesis_header_hash,
            &genesis_pk_tree_root,
        )
        .to_field_elements()
        .unwrap(),
    );

    inputs.append(
        &mut state_commitment(final_block_number, &final_header_hash, &final_pk_tree_root)
            .to_field_elements()
            .unwrap(),
    );

    inputs.append(
        &mut vk_commitment(verifying_key.clone())
            .to_field_elements()
            .unwrap(),
    );

    // Simulate proof.
    let toxic_waste = load_merger_wrapper_simulator(path).expect("Missing toxic waste.");
    let proof = toxic_waste.simulate_proof(&inputs, rng);
    ZKProof {
        block_number: block.block_number(),
        proof: Some(proof),
    }
}
