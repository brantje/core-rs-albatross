//! This crate contains a series of cryptographic primitives that are "off-circuit" versions of the primitives
//! in the zkp crate. The main purpose of these primitives is to be used by other crates who might
//! need an off-circuit version of a primitive that is however guaranteed to be consistent with the on-circuit
//! primitive used by one of our zk-SNARKs. They are also used for testing.

pub use macro_block::*;
pub use merkle_tree::*;
pub use pedersen::PEDERSEN_PARAMETERS;
pub use pk_tree::*;
pub use serialize::*;
pub use state_commitment::*;
pub use vk_commitment::*;

mod macro_block;
mod merkle_tree;
pub mod pedersen;
mod pk_tree;
mod serialize;
mod state_commitment;
mod vk_commitment;

use std::io;

use ark_relations::r1cs::SynthesisError;
use ark_serialize::SerializationError;
use beserial::SerializingError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NanoZKPError {
    #[error("filesystem error: {0}")]
    Filesystem(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] SerializationError),
    #[error("serialization error: {0}")]
    Serializing(#[from] SerializingError),
    #[error("circuit error: {0}")]
    Circuit(#[from] SynthesisError),
    #[error("empty proof")]
    EmptyProof,
}
