use std::io;

use log::level_filters::LevelFilter;
use nimiq_genesis::NetworkId;
use tracing_subscriber::{filter::Targets, prelude::*};

use nimiq_log::TargetsExt;
use nimiq_primitives::policy::{Policy, TEST_POLICY};
use nimiq_zkp::ZKP_VERIFYING_KEY;
use nimiq_zkp_component::prover_binary::prover_main;

/// This binary is only used in tests.
#[tokio::main]
async fn main() {
    initialize();
    log::info!("Starting proof generation");
    prover_main().await.unwrap();
}

fn initialize() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .with(
            Targets::new()
                .with_default(LevelFilter::INFO)
                .with_nimiq_targets(LevelFilter::DEBUG)
                .with_target("r1cs", LevelFilter::WARN)
                .with_env(),
        )
        .init();
    // Run tests with different policy values:
    // Shorter epochs and shorter batches
    let _ = Policy::get_or_init(TEST_POLICY);
    ZKP_VERIFYING_KEY.init_with_network_id(NetworkId::UnitAlbatross);
}
