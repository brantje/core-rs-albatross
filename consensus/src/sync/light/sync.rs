use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::task::Waker;

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt};

use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network, SubscribeEvents},
    request::RequestError,
};
use nimiq_primitives::task_executor::TaskExecutor;
use nimiq_zkp_component::{
    types::{Error, ZKPRequestEvent},
    zkp_component::ZKPComponentProxy,
};

use crate::{messages::Checkpoint, sync::syncer::MacroSync};

#[derive(Clone)]
/// This struct is used to request Epochs IDs (hashes) from other peers
/// in order to determine their macro chain state relative to us
pub(crate) struct EpochIds<T> {
    /// Indicates if the latest epoch id that was queried was found in the peer's chain
    pub locator_found: bool,
    /// The most recent epoch ids (hashes)
    pub ids: Vec<Blake2bHash>,
    /// The most recent checkpoint block in the latest epoch (if any)
    pub checkpoint: Option<Checkpoint>,
    /// Epoch number corresponding to the first hash in ids
    pub first_epoch_number: usize,
    /// The sender that created this struct
    pub sender: T,
}

impl<T> EpochIds<T> {
    #[inline]
    pub(crate) fn checkpoint_epoch_number(&self) -> usize {
        self.first_epoch_number + self.ids.len()
    }

    #[inline]
    pub(crate) fn last_epoch_number(&self) -> usize {
        self.checkpoint_epoch_number().saturating_sub(1)
    }
}

/// This struct is used to track all the macro requests sent to a particular peer
pub struct PeerMacroRequests {
    /// Number of requests that have been fulfilled
    completed_requests: usize,
    /// A Queue used to track the requests that have been sent, and their respective result
    queued_requests: VecDeque<(Blake2bHash, Option<Block>)>,
}
impl PeerMacroRequests {
    pub fn new() -> Self {
        Self {
            completed_requests: 0,
            queued_requests: VecDeque::new(),
        }
    }

    // Pushes a new request into the queue
    pub fn push_request(&mut self, block_hash: Blake2bHash) {
        self.queued_requests.push_back((block_hash, None))
    }

    // Pops a request from the queue
    pub fn pop_request(&mut self) -> Option<(Blake2bHash, Option<Block>)> {
        self.queued_requests.pop_front()
    }

    // Returns true if the request was updated, false in case the request was not found
    pub fn update_request(&mut self, block: Block) -> bool {
        let position = self
            .queued_requests
            .iter()
            .position(|(hash, _)| *hash == block.hash());

        if let Some(position) = position {
            if self.queued_requests[position].1.is_none() {
                // A fulfilled request is only count once
                self.completed_requests += 1;
            }
            // We update our block request.
            // Note: If we receive a response more than once, we use the latest
            let block_hash = block.hash();
            log::trace!(%block_hash, "Updating block request");
            self.queued_requests[position] = (block_hash, Some(block));

            true
        } else {
            log::trace!("Received a response for a block that we didn't expect");
            false
        }
    }

    // Returns true if all the requests have been completed
    pub fn is_ready(&self) -> bool {
        self.queued_requests.len() == self.completed_requests
    }
}

/// The LightMacroSync is one type of MacroSync and it is essentially a stream,
/// that operates on a per peer basis, emitting peers either as Outdated or Good.
/// To do this, it will:
///   1. Request the latest ZKP from a peer
///   2. Request epoch IDs from the peer
///   3. Request the last (if any) election or checkpoint blocks
/// If during the process, a peer is deemed as outdated, then it is emitted
pub struct LightMacroSync<TNetwork: Network> {
    /// The blockchain
    pub(crate) blockchain: BlockchainProxy,
    /// Reference to the network
    pub(crate) network: Arc<TNetwork>,
    /// Stream for peer joined and peer left events
    pub(crate) network_event_rx: SubscribeEvents<TNetwork::PeerId>,
    /// Used to track the macro requests on a per peer basis
    pub(crate) peer_requests: HashMap<TNetwork::PeerId, PeerMacroRequests>,
    /// The stream for epoch ids requests
    pub(crate) epoch_ids_stream:
        FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerId>>>>,
    /// Reference to the ZKP proxy used to interact with the ZKP component
    pub(crate) zkp_component_proxy: ZKPComponentProxy<TNetwork>,
    /// ZKP related requests (proofs)
    pub(crate) zkp_requests:
        FuturesUnordered<BoxFuture<'static, (Result<ZKPRequestEvent, Error>, TNetwork::PeerId)>>,
    /// Block requests
    pub(crate) block_headers: FuturesUnordered<
        BoxFuture<'static, (Result<Option<Block>, RequestError>, TNetwork::PeerId)>,
    >,
    /// Minimum distance to light sync in #blocks from the peers head.
    pub(crate) full_sync_threshold: u32,
    /// Task executor to be compatible with wasm and not wasm environments,
    pub(crate) executor: Box<dyn TaskExecutor + Send + 'static>,
    /// Waker used for the poll next function
    pub(crate) waker: Option<Waker>,
}

impl<TNetwork: Network> LightMacroSync<TNetwork> {
    pub fn new(
        blockchain: BlockchainProxy,
        network: Arc<TNetwork>,
        network_event_rx: SubscribeEvents<TNetwork::PeerId>,
        zkp_component_proxy: ZKPComponentProxy<TNetwork>,
        full_sync_threshold: u32,
        executor: impl TaskExecutor + Send + 'static,
    ) -> Self {
        Self {
            blockchain,
            network,
            network_event_rx,
            peer_requests: HashMap::new(),
            epoch_ids_stream: FuturesUnordered::new(),
            zkp_component_proxy,
            zkp_requests: FuturesUnordered::new(),
            waker: None,
            executor: Box::new(executor),
            full_sync_threshold,
            block_headers: Default::default(),
        }
    }

    pub fn peers(&self) -> impl Iterator<Item = &TNetwork::PeerId> {
        self.peer_requests.keys()
    }

    pub fn remove_peer_requests(&mut self, peer_id: TNetwork::PeerId) {
        self.peer_requests.remove(&peer_id);
    }

    pub fn disconnect_peer(&mut self, peer_id: TNetwork::PeerId, reason: CloseReason) {
        // Remove all pending peer requests (if any)
        self.remove_peer_requests(peer_id);
        let network = Arc::clone(&self.network);
        // We disconnect from this peer
        self.executor.exec(Box::pin({
            async move {
                network.disconnect_peer(peer_id, reason).await;
            }
        }));
    }
}

impl<TNetwork: Network> MacroSync<TNetwork::PeerId> for LightMacroSync<TNetwork> {
    fn add_peer(&self, peer_id: TNetwork::PeerId) {
        info!("Requesting zkp from peer: {:?}", peer_id);

        self.zkp_requests
            .push(Self::request_zkps(self.zkp_component_proxy.clone(), peer_id).boxed());

        // Pushing the future to FuturesUnordered above does not wake the task that
        // polls `epoch_ids_stream`. Therefore, we need to wake the task manually.
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}
