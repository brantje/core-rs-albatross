use std::ops::RangeInclusive;

use thiserror::Error;

use nimiq_utils::math::log2;

use crate::contribution::AggregatableContribution;

/// Errors that can happen during partitioning
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PartitioningError {
    #[error("Invalid level: {level}")]
    InvalidLevel { level: usize },
    #[error("Empty level: {level}")]
    EmptyLevel { level: usize },
}

pub trait Partitioner: Send + Sync {
    /// Number of levels
    fn levels(&self) -> usize;

    /// Total number of identities
    fn size(&self) -> usize;

    /// Number of identities at `level`
    fn level_size(&self, level: usize) -> usize;

    /// Range of identities that need to be contacted at `level`
    fn range(&self, level: usize) -> Result<RangeInclusive<usize>, PartitioningError>;

    /// Combine `AggregatableContributions` to a new `AggregatableContribution` for next level
    /// TODO: Return `Result<C, PartitioningError>` instead of option
    fn combine<C: AggregatableContribution>(&self, signatures: Vec<&C>, level: usize) -> Option<C>;
}

/// The next level is always double the size of the current level
#[derive(Clone, Debug)]
pub struct BinomialPartitioner {
    /// The ID of the node itself
    node_id: usize,

    /// The number of IDs handled (i.e. `max_id + 1`)
    num_ids: usize,

    /// The number of levels
    num_levels: usize,
}

impl BinomialPartitioner {
    pub fn new(node_id: usize, num_ids: usize) -> Self {
        let num_levels = match num_ids {
            0 => panic!("num_ids must be greater than 0"),
            1 => 1,
            n => log2(n - 1) + 2,
        };
        assert!(node_id < num_ids);
        Self {
            node_id,
            num_ids,
            num_levels,
        }
    }
}

impl Partitioner for BinomialPartitioner {
    /// returns the number of levels including level 0 (which is always this nodes own contribution)
    fn levels(&self) -> usize {
        self.num_levels
    }

    fn size(&self) -> usize {
        self.num_ids
    }

    fn level_size(&self, level: usize) -> usize {
        if let Ok(range) = self.range(level) {
            (range.end() - range.start()) + 1
        } else {
            0
        }
    }

    fn range(&self, level: usize) -> Result<RangeInclusive<usize>, PartitioningError> {
        if level == 0 {
            Ok(self.node_id..=self.node_id)
        } else if level >= self.num_levels {
            Err(PartitioningError::InvalidLevel { level })
        } else {
            // mask for bits which cover the range
            let m = (1 << (level - 1)) - 1;
            // bit that must be flipped
            let f = 1 << (level - 1);

            let min = (self.node_id ^ f) & !m;
            let max = std::cmp::min((self.node_id ^ f) | m, self.num_ids - 1);

            if min > max {
                Err(PartitioningError::EmptyLevel { level })
            } else {
                Ok(min..=max)
            }
        }
    }

    /// TODO: Why do we have `_level` as argument?
    fn combine<C: AggregatableContribution>(
        &self,
        contributions: Vec<&C>,
        _level: usize,
    ) -> Option<C> {
        let mut combined = (*contributions.first()?).clone();

        for contribution in contributions.iter().skip(1) {
            combined
                .combine(contribution)
                .unwrap_or_else(|e| panic!("Failed to combine contributions: {e}"));
        }

        Some(combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimiq_test_log::test;
    use rand::Rng;

    #[test]
    fn test_partitioner() {
        /*
        partitioner: node_id = 3
            ---ID---   -Level-
            0    000   . . 2 .
            1    001   . . 2 .
            2    010   . 1 . .
            3    011   0 . . .
            4    100   . . . 3
            5    101   . . . 3
            6    110   . . . 3
            7    111   . . . 3

        level = 3
        m = (1 << level - 1) - 1 = 100 - 1 = 011
        f = (1 << level)                   = 100

        other_partitioner: node_id = 1
            ---ID---   -Level-
            0    000   . 1 . .
            1    001   0 . . .
            2    010   . . 2 .
            3    011   . . 2 .
            4    100   . . . 3
            5    101   . . . 3
            6    110   . . . 3
            7    111   . . . 3
        */

        let partitioner = BinomialPartitioner::new(3, 8);
        let second_partitioner = BinomialPartitioner::new(1, 8);
        let third_partitioner = BinomialPartitioner::new(7, 8);

        assert_eq!(partitioner.levels(), 4);

        assert_eq!(partitioner.range(0), Ok(3..=3), "Level 0");
        assert_eq!(second_partitioner.range(0), Ok(1..=1), "Level 0");

        assert_eq!(partitioner.range(1), Ok(2..=2), "Level 1");
        assert_eq!(second_partitioner.range(1), Ok(0..=0), "Level 1");

        assert_eq!(partitioner.range(2), Ok(0..=1), "Level 2");
        assert_eq!(second_partitioner.range(2), Ok(2..=3), "Level 2");

        assert_eq!(partitioner.range(3), Ok(4..=7), "Level 3");
        assert_eq!(second_partitioner.range(3), Ok(4..=7), "Level 3");

        // must be symetrical
        for level in 2..partitioner.levels() {
            if partitioner
                .range(level)
                .unwrap()
                .contains(&second_partitioner.node_id)
            {
                assert!(second_partitioner
                    .range(level)
                    .unwrap()
                    .contains(&partitioner.node_id));
            }
            if partitioner
                .range(level)
                .unwrap()
                .contains(&third_partitioner.node_id)
            {
                assert!(third_partitioner
                    .range(level)
                    .unwrap()
                    .contains(&partitioner.node_id));
            }
        }

        assert_eq!(
            partitioner.range(4),
            Err(PartitioningError::InvalidLevel { level: 4 })
        );
    }

    #[test]
    fn test_non_power_of_two_levels() {
        assert_eq!(BinomialPartitioner::new(0, 7).levels(), 4);
        assert_eq!(BinomialPartitioner::new(0, 6).levels(), 4);
        assert_eq!(BinomialPartitioner::new(0, 5).levels(), 4);
        assert_eq!(BinomialPartitioner::new(0, 4).levels(), 3);
    }

    #[test]
    fn test_partitioner_non_power_of_two() {
        /*
        partitioner: node_id = 5
            ---ID---   -Level-
            0   0000   . . . 3 .
            1   0001   . . . 3 .
            2   0010   . . . 3 .
            3   0011   . . . 3 .
            4   0100   . 1 . . .
            5   0101   0 . . . .
            6   0110   . . 2 . .
            7   0111   . . 2 . .
            8   1000   . . . . 4
            9   1001   . . . . 4

        level = 4

        other_partitioner: node_id = 9
            ---ID---   -Level-
            0   0000   . . . . 4
            1   0001   . . . . 4
            2   0010   . . . . 4
            3   0011   . . . . 4
            4   0100   . . . . 4
            5   0101   . . . . 4
            6   0110   . . . . 4
            7   0111   . . . . 4
            8   1000   . 1 . . .
            9   1001   0 . . . .
        */

        let partitioner = BinomialPartitioner::new(5, 10);
        let second_partitioner = BinomialPartitioner::new(9, 10);

        assert_eq!(partitioner.levels(), 5);

        assert_eq!(partitioner.range(0), Ok(5..=5), "Level 0");
        assert_eq!(second_partitioner.range(0), Ok(9..=9), "Level 0");

        assert_eq!(partitioner.range(1), Ok(4..=4), "Level 1");
        assert_eq!(second_partitioner.range(1), Ok(8..=8), "Level 1");

        // Note that in some cases, we would get ranges that correspond to ids outside of the range of num_ids
        // I.e: we have some sparse subtrees
        assert_eq!(partitioner.range(2), Ok(6..=7), "Level 2");
        assert_eq!(
            second_partitioner.range(2),
            Err(PartitioningError::EmptyLevel { level: 2 }),
            "Level 2"
        );

        assert_eq!(partitioner.range(3), Ok(0..=3), "Level 3");
        assert_eq!(
            second_partitioner.range(3),
            Err(PartitioningError::EmptyLevel { level: 3 }),
            "Level 3"
        );

        assert_eq!(partitioner.range(4), Ok(8..=9), "Level 4");
        assert_eq!(second_partitioner.range(4), Ok(0..=7), "Level 4");

        for level in 2..partitioner.levels() {
            if partitioner
                .range(level)
                .unwrap()
                .contains(&second_partitioner.node_id)
            {
                assert!(second_partitioner
                    .range(level)
                    .unwrap()
                    .contains(&partitioner.node_id));
            }
        }

        assert_eq!(
            partitioner.range(5),
            Err(PartitioningError::InvalidLevel { level: 5 })
        );
    }

    #[test]
    fn test_symmetry() {
        let mut rng = rand::thread_rng();
        let num_ids = rng.gen_range(8..512);

        let node_id = rng.gen_range(0..num_ids);
        let second_node_id = rng.gen_range(0..num_ids);

        log::debug!(num_ids, node_id, second_node_id);

        let partitioner = BinomialPartitioner::new(node_id, num_ids);
        let second_partitioner = BinomialPartitioner::new(second_node_id, num_ids);

        assert_eq!(partitioner.levels(), log2(num_ids - 1) + 2);
        let mut total_peers = 1; // In the for loop below we skip level 0 (that always have a peer)

        for level in 1..partitioner.levels() {
            if let Ok(range) = partitioner.range(level) {
                let range_len: usize = (range.end() - range.start()) + 1;

                // Some of the levels might be shorter than 2^(level-1) so we can't know the exact size unless
                // we do the same bitwise ops
                assert!(
                    range_len <= u32::pow(2, (level - 1).try_into().unwrap()) as usize
                        && range_len <= num_ids - total_peers,
                );

                total_peers += range_len;

                if range.contains(&second_partitioner.node_id) {
                    assert!(second_partitioner
                        .range(level)
                        .unwrap()
                        .contains(&partitioner.node_id));
                }
            }
        }
    }
}
