use beserial::SerializingError;
use thiserror::Error;

/// An enum containing possible errors that can happen in the Merkle Radix Trie.
#[derive(Error, Debug, Eq, PartialEq)]
pub enum MerkleRadixTrieError {
    #[error("Prefix doesn't match node's key.")]
    WrongPrefix,
    #[error("Tried to query the value of a branch node. Branch nodes don't have a value.")]
    BranchesHaveNoValue,
    #[error("Tried to query a child that does not exist.")]
    ChildDoesNotExist,
    #[error("Child is incomplete.")]
    ChildIsStump,
    #[error("Tried to store a value at the root node.")]
    RootCantHaveValue,
    #[error("Failed to (de)serialize a value.")]
    SerializationFailed(SerializingError),
    #[error("Tree is already complete.")]
    TrieAlreadyComplete,
    #[error("Chunk does not match tree state.")]
    NonMatchingChunk,
    #[error("Root hash does not match expected hash after applying chunk.")]
    ChunkHashMismatch,
    #[error("Chunk is invalid: {0}")]
    InvalidChunk(&'static str),
    #[error("Trie is not complete")]
    IncompleteTrie,
}

impl From<SerializingError> for MerkleRadixTrieError {
    fn from(err: SerializingError) -> Self {
        MerkleRadixTrieError::SerializationFailed(err)
    }
}
