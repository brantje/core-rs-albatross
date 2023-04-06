use std::collections::BTreeMap;
use std::sync::Arc;

use crate::identity::IdentityRegistry;
use crate::partitioner::Partitioner;
use crate::protocol::Protocol;
use crate::{contribution::AggregatableContribution, identity::Identity};
use nimiq_collections::bitset::BitSet;

pub trait ContributionStore<TId, TProtocol>: Send + Sync
where
    TId: std::fmt::Debug + Clone + 'static,
    TProtocol: Protocol<TId>,
{
    /// Put `signature` into the store for level `level`.
    fn put(
        &mut self,
        contribution: TProtocol::Contribution,
        level: usize,
        registry: Arc<TProtocol::Registry>,
        identifier: TId,
    );

    /// Return the number of the current best level.
    fn best_level(&self) -> usize;

    /// Check whether we have an individual signature from a certain `peer_id`.
    fn individual_received(&self, peer_id: usize) -> bool;

    /// Return a `BitSet` of the verified individual signatures we have for a given `level`.
    ///
    /// Panics if `level` is invalid.
    fn individual_verified(&self, level: usize) -> &BitSet;

    /// Returns a `BTreeMap` of the individual signatures for the given `level`.
    ///
    /// Panics if `level` is invalid.
    fn individual_signature(
        &self,
        level: usize,
        peer_id: usize,
    ) -> Option<&TProtocol::Contribution>;

    /// Returns the best multi-signature for the given `level`.
    ///
    /// Panics if `level` is invalid.
    fn best(&self, level: usize) -> Option<&TProtocol::Contribution>;

    /// Returns the best combined multi-signature for all levels upto `level`.
    fn combined(&self, level: usize) -> Option<TProtocol::Contribution>;
}

#[derive(Clone, Debug)]
/// An implementation of the `ContributionStore` trait
pub struct ReplaceStore<TId: Clone + std::fmt::Debug + 'static, TProtocol: Protocol<TId>> {
    /// The Partitioner used to create the Aggregation Tree.
    partitioner: Arc<TProtocol::Partitioner>,

    /// The currently best level
    best_level: usize,

    /// BitSet that contains the IDs of all individual contributions we already received
    individual_received: BitSet,

    /// BitSets for all the individual contributions that we already verified
    /// level -> peer_id -> bool
    individual_verified: Vec<BitSet>,

    /// All individual contributions
    /// level -> peer_id -> Contribution
    individual_contributions: Vec<BTreeMap<usize, (TProtocol::Contribution, Identity)>>,

    /// The best Contribution at each level
    best_contribution: BTreeMap<usize, (TProtocol::Contribution, Identity)>,
}

impl<TId: Clone + std::fmt::Debug + 'static, TProtocol: Protocol<TId>>
    ReplaceStore<TId, TProtocol>
{
    /// Create a new Replace Store using the Partitioner `partitioner`.
    pub fn new(partitioner: Arc<TProtocol::Partitioner>) -> Self {
        let n = partitioner.size();
        let mut individual_verified = Vec::with_capacity(partitioner.levels());
        let mut individual_signatures = Vec::with_capacity(partitioner.levels());
        for level in 0..partitioner.levels() {
            individual_verified.push(BitSet::with_capacity(partitioner.level_size(level)));
            individual_signatures.push(BTreeMap::new())
        }

        Self {
            partitioner,
            best_level: 0,
            individual_received: BitSet::with_capacity(n),
            individual_verified,
            individual_contributions: individual_signatures,
            best_contribution: BTreeMap::new(),
        }
    }

    fn check_merge(
        &self,
        contribution: &TProtocol::Contribution,
        registry: Arc<TProtocol::Registry>,
        level: usize,
        identifier: TId,
    ) -> Option<TProtocol::Contribution> {
        if let Some((best_contribution, identity)) = self.best_contribution.get(&level) {
            let best_contributors = identity.as_bitset();
            trace!(
                id = ?identifier.clone(),
                ?best_contribution,
                ?identity,
                level,
                "Current best for level"
            );

            // try to combine
            let mut contribution = contribution.clone();

            // If the combining fails, it is due to the contributions having an overlap.
            // One may be the superset of the other which makes it the strictly better set.
            // If that is the case the better can immediately be returned.
            // Otherwise individual signatures must still be checked.
            let contributors = if let Err(e) = contribution.combine(best_contribution) {
                // The contributors of contribution represented as Identities.
                let contributors = registry
                    .signers_identity(&contribution.contributors())
                    .as_bitset();

                // merging failed. Check if contribution is a superset of best_contribution.
                if contributors.is_superset(&best_contributors) {
                    trace!(
                        id = ?identifier.clone(),
                        ?contribution,
                        ?best_contribution,
                        "New signature is superset of current best. Replacing",
                    );
                    return Some(contribution);
                } else {
                    trace!(
                        id = ?identifier.clone(),
                        ?contribution,
                        ?best_contribution,
                        error = ?e,
                        "Combining contributions failed",
                    );
                    contributors
                }
            } else {
                registry
                    .signers_identity(&contribution.contributors())
                    .as_bitset()
            };

            // Individual signatures of identities which this node has that are already verified.
            let individual_verified = self.individual_verified.get(level).unwrap_or_else(|| {
                panic!("Individual verified contributions BitSet is missing for level {level}")
            });

            // the bits set here are verified individual signatures that can be added to `contribution`
            let complements = &(&contributors & individual_verified) ^ individual_verified;

            // check that if we combine we get a better signature
            if complements.len() + contributors.len() <= best_contributors.len() {
                // This should not be observed really as the evaluator should filter signatures which cannot provide
                // improvements out.
                trace!(
                    id = ?identifier.clone(),
                    "No improvement possible",
                );
                None
            } else {
                // put in the individual signatures
                for id in complements.iter() {
                    // get individual signature
                    let individual = self
                        .individual_contributions
                        .get(level)
                        .unwrap_or_else(|| {
                            panic!("Individual contribution missing for level {level}")
                        })
                        .get(&id)
                        .unwrap_or_else(|| {
                            panic!("Individual contribution {id} missing for level {level}")
                        });

                    // merge individual signature into multisig
                    contribution
                        .combine(&individual.0)
                        .unwrap_or_else(|e| panic!("Individual contribution from id={id} can't be added to aggregate contributions: {e}"));
                }

                Some(contribution)
            }
        } else {
            // This is normal whenever the first signature for a level is processed.
            trace!(
                id = ?identifier.clone(),
                ?level,
                contributors = ?contribution.contributors(),
                "Level was empty",
            );
            Some(contribution.clone())
        }
    }
}

impl<TId, TProtocol> ContributionStore<TId, TProtocol> for ReplaceStore<TId, TProtocol>
where
    TId: Clone + std::fmt::Debug + 'static,
    TProtocol: Protocol<TId>,
{
    fn put(
        &mut self,
        contribution: TProtocol::Contribution,
        level: usize,
        registry: Arc<TProtocol::Registry>,
        identifier: TId,
    ) {
        if let Identity::Single(id) = registry.signers_identity(&contribution.contributors()) {
            self.individual_verified
                .get_mut(level)
                .unwrap_or_else(|| panic!("Missing Level {level}"))
                .insert(id);
            self.individual_contributions
                .get_mut(level)
                .unwrap_or_else(|| panic!("Missing Level {level}"))
                .insert(id, (contribution.clone(), Identity::Single(id)));
        }

        if let Some(best_contribution) = self.check_merge(
            &contribution,
            Arc::clone(&registry),
            level,
            identifier.clone(),
        ) {
            let best_identity = registry.signers_identity(&best_contribution.contributors());
            self.best_contribution
                .insert(level, (best_contribution, best_identity));
            if level > self.best_level {
                trace!(
                    id = ?identifier,
                    ?level,
                    "best level is now",
                );
                self.best_level = level;
            }
        }
    }

    fn best_level(&self) -> usize {
        self.best_level
    }

    fn individual_received(&self, peer_id: usize) -> bool {
        self.individual_received.contains(peer_id)
    }

    fn individual_verified(&self, level: usize) -> &BitSet {
        self.individual_verified
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {level}"))
    }

    fn individual_signature(
        &self,
        level: usize,
        peer_id: usize,
    ) -> Option<&TProtocol::Contribution> {
        self.individual_contributions
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {level}"))
            .get(&peer_id)
            .map(|(c, _)| c)
    }

    fn best(&self, level: usize) -> Option<&TProtocol::Contribution> {
        self.best_contribution.get(&level).map(|(c, _)| c)
    }

    fn combined(&self, mut level: usize) -> Option<TProtocol::Contribution> {
        // TODO: Cache this?

        let mut signatures = Vec::new();
        for (_, (signature, _)) in self.best_contribution.range(0..=level) {
            signatures.push(signature)
        }

        self.partitioner.combine(signatures, level)
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::RwLock;
    use rand::Rng;

    use beserial::{Deserialize, Serialize};
    use nimiq_test_log::test;

    use super::*;
    use crate::{contribution::ContributionError, partitioner::BinomialPartitioner};

    /// Dumb Aggregate adding numbers.
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct Contribution {
        value: u64,
        contributors: BitSet,
    }

    impl AggregatableContribution for Contribution {
        fn contributors(&self) -> BitSet {
            self.contributors.clone()
        }

        fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError> {
            let overlap = &self.contributors & &other_contribution.contributors;
            // only contributions without any overlap can be combined.
            if overlap.is_empty() {
                // the combined value is the addition of the 2 individual values
                self.value += other_contribution.value;
                // the contributors of the resulting contribution are the combined sets of both individual contributions.
                self.contributors = &self.contributors | &other_contribution.contributors;
                Ok(())
            } else {
                Err(ContributionError::Overlapping(overlap))
            }
        }
    }

    #[test]
    fn it_can_combine_contributions() {
        let mut rng = rand::thread_rng();
        let num_ids = rng.gen_range(8..512);

        let node_id = rng.gen_range(0..num_ids);

        log::debug!(num_ids, node_id);

        // Create the partitions
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_ids));

        let store = Arc::new(RwLock::new(
            ReplaceStore::<BinomialPartitioner, Contribution>::new(partitioner.clone()),
        ));

        // Define a level that is going to be used for the contributions
        let level = rng.gen_range(0..partitioner.levels()) as usize;
        let single_identity = Identity::Single(0);

        // Create the first contribution
        let mut first_contributors = BitSet::new();
        first_contributors.insert(1);

        let first_contribution = Contribution {
            value: 1,
            contributors: first_contributors,
        };

        store
            .write()
            .put(first_contribution.clone(), level, single_identity);

        {
            let store = store.read();
            let current_best_contribution = store.best(level);

            if let Some(best) = current_best_contribution {
                assert_eq!(first_contribution, *best);
                log::debug!(?best, "Current best contribution");
            }
        }

        // Create a second contribution, using a different identity and a different contributor
        let mut second_contributors = BitSet::new();
        second_contributors.insert(2);

        let single_identity = Identity::Single(1);

        let second_contribution = Contribution {
            value: 10,
            contributors: second_contributors,
        };

        store
            .write()
            .put(second_contribution.clone(), level, single_identity);

        {
            let store = store.read();
            let current_best_contribution = store.best(level);

            if let Some(best) = current_best_contribution {
                // Now the best contribution should be the aggregated one
                let mut contributors = BitSet::new();
                contributors.insert(1);
                contributors.insert(2);

                let aggregated_contribution = Contribution {
                    value: 11,
                    contributors,
                };

                log::debug!(?best, "Current best contribution");
                assert_eq!(aggregated_contribution, *best);
            }
        }

        // Now try to insert a contribution using the same identity (1)
        // Note that since we are using a previous identity, then we need to subtract
        // the previous contribution from this identity to the value to be aggregated
        let mut third_contributors = BitSet::new();
        // Note that we are using a different contributor number
        third_contributors.insert(8);

        let single_identity = Identity::Single(1);

        let second_contribution = Contribution {
            value: 20,
            contributors: third_contributors,
        };

        store
            .write()
            .put(second_contribution.clone(), level, single_identity);

        {
            let store = store.read();
            let current_best_contribution = store.best(level);

            if let Some(best) = current_best_contribution {
                // Now the best contribution should be the aggregated one
                let mut contributors = BitSet::new();
                contributors.insert(1);
                contributors.insert(8);

                let aggregated_contribution = Contribution {
                    value: 21,
                    contributors,
                };
                assert_eq!(aggregated_contribution, *best);

                log::debug!(?best, "Final best contribution");
            }
        }
    }
}
