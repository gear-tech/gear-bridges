use serde::{Deserialize, Serialize};

use super::*;

pub struct Messages(Vec<accumulator::Request>);

impl Messages {
    pub fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    fn compare(
        authority_set_id: AuthoritySetId,
        block_number: GearBlockNumber,
        authority_set_id_new: AuthoritySetId,
        block_number_new: GearBlockNumber,
    ) -> Ordering {
        if authority_set_id == authority_set_id_new {
            return block_number.cmp(&block_number_new);
        }

        authority_set_id.cmp(&authority_set_id_new)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    // None -> the inner vector is full so the message is rejected
    pub fn add(&mut self, message_new: accumulator::Request) -> Option<()> {
        if self.0.len() >= self.0.capacity() {
            return None;
        }

        match self.0.binary_search_by(|message| {
            Self::compare(
                message.authority_set_id,
                message.block,
                message_new.authority_set_id,
                message_new.block,
            )
        }) {
            Ok(i) | Err(i) => self.0.insert(i, message_new),
        }

        Some(())
    }

    pub fn drain_all(
        &mut self,
        merkle_root: &RelayedMerkleRoot,
    ) -> Drain<'_, accumulator::Request> {
        let index_end = match self.0.binary_search_by(|message| {
            Self::compare(
                message.authority_set_id,
                message.block,
                merkle_root.authority_set_id,
                merkle_root.block,
            )
        }) {
            Ok(i) => i + 1,
            Err(i) => i,
        };

        let index_start = match self.0.binary_search_by(|message| {
            Self::compare(
                message.authority_set_id,
                message.block,
                merkle_root.authority_set_id,
                GearBlockNumber(0),
            )
        }) {
            Ok(i) | Err(i) => i,
        };

        self.0.drain(index_start..index_end)
    }
    pub fn drain(
        &mut self,
        merkle_root: &RelayedMerkleRoot,
        timestamp: u64,
        delay: impl Fn(ActorId) -> u64,
    ) -> impl Iterator<Item = accumulator::Request> {
        let mut removed = Vec::new();
        self.0.retain(|message| {
            if message.authority_set_id == merkle_root.authority_set_id
                && message.block <= merkle_root.block
                && timestamp >= merkle_root.timestamp + delay(message.source)
            {
                removed.push(message.clone());
                false
            } else {
                true
            }
        });
        removed.into_iter()
    }

    pub fn drain_timestamp(
        &mut self,
        timestamp: u64,
        delay: impl Fn(ActorId) -> u64,
        merkle_roots: &MerkleRoots,
    ) -> impl Iterator<Item = (RelayedMerkleRoot, accumulator::Request)> {
        let mut removed = Vec::new();

        self.0.retain(|message| {
            let delay = delay(message.source);
            if let Some(root) =
                merkle_roots.find(message.authority_set_id, message.block, timestamp, delay)
            {
                removed.push((*root, message.clone()));
                false
            } else {
                true
            }
        });

        removed.into_iter()
    }
}

/// Represents the successful status of adding a relayed merkle root.
#[derive(Clone, Debug)]
pub enum Added {
    /// Provided instance is new and added.
    Ok,
    /// Returns the oldest evicted root, or the highest root of an evicted authority.
    Removed(RelayedMerkleRoot),
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct MerkleRoots {
    roots: Vec<RelayedMerkleRoot>,
    #[serde(skip)]
    capacity: usize,
}

impl MerkleRoots {
    // Bound both authority history and roots per authority; preserve older authority coverage.
    pub fn new(capacity: usize) -> Self {
        Self {
            roots: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn compare(
        authority_set_id: AuthoritySetId,
        block_number: GearBlockNumber,
        authority_set_id_new: AuthoritySetId,
        block_number_new: GearBlockNumber,
    ) -> Ordering {
        authority_set_id_new
            .cmp(&authority_set_id)
            .then_with(|| block_number_new.cmp(&block_number))
    }

    pub fn find(
        &self,
        authority_set_id: AuthoritySetId,
        block_number: GearBlockNumber,
        last_timestamp: u64,
        delay: u64,
    ) -> Option<&RelayedMerkleRoot> {
        let end = match self.roots.binary_search_by(|root| {
            Self::compare(
                root.authority_set_id,
                root.block,
                authority_set_id,
                block_number,
            )
        }) {
            Ok(i) => i + 1,
            Err(i) => i,
        };
        self.roots[..end]
            .iter()
            .rev()
            .take_while(|root| root.authority_set_id == authority_set_id)
            .find(|root| {
                root.timestamp
                    .checked_add(delay)
                    .is_some_and(|at| last_timestamp >= at)
            })
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&RelayedMerkleRoot> {
        self.roots.get(i)
    }

    // Err(i): duplicate identity or a root older than the retained history bound.
    pub fn add(&mut self, root_new: RelayedMerkleRoot) -> Result<Added, usize> {
        let i = match self.roots.binary_search_by(|root| {
            Self::compare(
                root.authority_set_id,
                root.block,
                root_new.authority_set_id,
                root_new.block,
            )
        }) {
            Ok(i) => return Err(i),
            Err(i) => i,
        };
        let start = self
            .roots
            .partition_point(|root| root.authority_set_id > root_new.authority_set_id);
        let end = self
            .roots
            .partition_point(|root| root.authority_set_id >= root_new.authority_set_id);
        if start != end && end - start >= self.capacity {
            if i == end {
                return Err(i);
            }
            self.roots[i..end].rotate_right(1);
            return Ok(Added::Removed(std::mem::replace(
                &mut self.roots[i],
                root_new,
            )));
        }
        let mut removed = None;
        if start == end {
            let authorities = usize::from(!self.roots.is_empty())
                + self
                    .roots
                    .windows(2)
                    .filter(|pair| pair[0].authority_set_id != pair[1].authority_set_id)
                    .count();
            if authorities >= self.capacity {
                if i == self.roots.len() {
                    return Err(i);
                }
                let oldest = self
                    .roots
                    .last()
                    .expect("retained authority exists")
                    .authority_set_id;
                let cutoff = self
                    .roots
                    .partition_point(|root| root.authority_set_id > oldest);
                removed = Some(self.roots[cutoff]);
                self.roots.truncate(cutoff);
            }
        }
        self.roots.insert(i, root_new);
        Ok(removed.map_or(Added::Ok, Added::Removed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    #[test]
    fn bounded_root_history_preserves_authority_coverage() {
        let root = RelayedMerkleRoot {
            block: GearBlockNumber(100),
            block_hash: [1; 32].into(),
            authority_set_id: AuthoritySetId(1),
            merkle_root: [2; 32].into(),
            timestamp: 1000,
        };
        let newer = RelayedMerkleRoot {
            block: GearBlockNumber(110),
            timestamp: 1100,
            ..root
        };
        let newest = RelayedMerkleRoot {
            block: GearBlockNumber(120),
            timestamp: 1200,
            ..root
        };
        let mut roots = MerkleRoots::new(2);
        roots.add(root).unwrap();
        roots.add(newer).unwrap();
        assert_eq!(
            roots.find(root.authority_set_id, root.block, 1300, 300),
            Some(&root)
        );
        assert_eq!(
            roots.find(root.authority_set_id, GearBlockNumber(101), 1300, 300),
            None
        );
        assert!(matches!(roots.add(newest), Ok(Added::Removed(value)) if value == root));
        assert!(roots.add(root).is_err());
        assert!(roots.add(newer).is_err());
        assert_eq!(
            roots.find(root.authority_set_id, root.block, 1400, 300),
            Some(&newer)
        );
        let next_authority = RelayedMerkleRoot {
            authority_set_id: AuthoritySetId(2),
            ..newest
        };
        roots.add(next_authority).unwrap();
        assert_eq!(
            roots.find(root.authority_set_id, root.block, 1400, 300),
            Some(&newer)
        );
        // Hydrate the unchanged JSON array through the bounded owner path used at startup.
        let saved: MerkleRoots =
            serde_json::from_str(&serde_json::to_string(&roots).unwrap()).unwrap();
        let mut restored = MerkleRoots::new(2);
        for i in 0..saved.len() {
            restored.add(*saved.get(i).unwrap()).unwrap();
        }
        assert_eq!(
            restored.find(root.authority_set_id, root.block, 1400, 300),
            Some(&newer)
        );
        let last_authority = RelayedMerkleRoot {
            authority_set_id: AuthoritySetId(3),
            ..newest
        };
        assert!(
            matches!(restored.add(last_authority), Ok(Added::Removed(value)) if value == newest)
        );
        assert_eq!(
            restored.find(root.authority_set_id, root.block, u64::MAX, 0),
            None
        );
        assert!(restored.add(root).is_err());
        assert_eq!(
            restored.find(
                next_authority.authority_set_id,
                next_authority.block,
                1500,
                300
            ),
            Some(&next_authority)
        );
    }

    #[test]
    fn messages() {
        let root = RelayedMerkleRoot {
            block: GearBlockNumber(16_881_826),
            block_hash: hex!("410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848")
                .into(),
            authority_set_id: AuthoritySetId(1_183),
            merkle_root: hex!("bfd87951376d18fe27f106b603a4f082d83f0cc4da3c5bebb61ab276cb8033fe")
                .into(),
            timestamp: 0,
        };
        let data = [
            accumulator::Request {
                block: GearBlockNumber(16_883_172),
                block_hash: hex!(
                    "38f753a5d02c81e91ff8b3950c2cd03c526ced9abc0b6ef29803ee4250a0df85"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_180),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
            accumulator::Request {
                block: GearBlockNumber(16_883_289),
                block_hash: hex!(
                    "74dcef50f0cf4299a0774b147a748f3d5961d913afb9d0e74868a298255edea2"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
            accumulator::Request {
                block: GearBlockNumber(16_881_824),
                block_hash: hex!(
                    "24b437d833fc6b7e9aea9d987ca1411ae293d340427da57dd7d9888fda8b16a2"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
            accumulator::Request {
                block: GearBlockNumber(16_883_636),
                block_hash: hex!(
                    "410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_184),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
        ];

        let mut messages = Messages::new(data.len());
        assert!(messages.drain_all(&root).collect::<Vec<_>>().is_empty());

        assert!(messages.add(data.first().unwrap().clone()).is_some());
        assert!(messages.add(data.get(3).unwrap().clone()).is_some());
        assert!(messages.drain_all(&root).collect::<Vec<_>>().is_empty());

        let mut messages = Messages::new(data.len());
        for message in &data {
            assert!(messages.add(message.clone()).is_some());
        }

        assert!(messages.add(data[0].clone()).is_none());

        let mut removed = messages.drain_all(&root).collect::<Vec<_>>();
        let removed_message = removed.pop();
        assert!(
            matches!(removed_message, Some(ref message) if removed.is_empty() && message == &data[2]),
            "removed = {removed:?}, removed_message = {removed_message:?}, data[2] = {:?}",
            data[2]
        );
        assert_eq!(messages.0.len(), data.len() - 1);
    }

    #[test]
    fn messages_drain_with_timestamps_and_delays() {
        let actor_fast = ActorId::from([1; 32]);
        let actor_medium = ActorId::from([2; 32]);
        let actor_slow = ActorId::from([3; 32]);

        let base_timestamp = 1000u64;

        let root = RelayedMerkleRoot {
            block: GearBlockNumber(16_881_826),
            block_hash: hex!("410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848")
                .into(),
            authority_set_id: AuthoritySetId(1_183),
            merkle_root: hex!("bfd87951376d18fe27f106b603a4f082d83f0cc4da3c5bebb61ab276cb8033fe")
                .into(),
            timestamp: base_timestamp,
        };

        let data = [
            accumulator::Request {
                block: GearBlockNumber(16_881_800),
                block_hash: hex!(
                    "38f753a5d02c81e91ff8b3950c2cd03c526ced9abc0b6ef29803ee4250a0df85"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                tx_uuid: Uuid::now_v7(),
                source: actor_fast, // Should be drained with delay=100
            },
            accumulator::Request {
                block: GearBlockNumber(16_881_825),
                block_hash: hex!(
                    "74dcef50f0cf4299a0774b147a748f3d5961d913afb9d0e74868a298255edea2"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                tx_uuid: Uuid::now_v7(),
                source: actor_medium, // Should NOT be drained with delay=500
            },
            accumulator::Request {
                block: GearBlockNumber(16_881_820),
                block_hash: hex!(
                    "24b437d833fc6b7e9aea9d987ca1411ae293d340427da57dd7d9888fda8b16a2"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                tx_uuid: Uuid::now_v7(),
                source: actor_slow, // Should NOT be drained with delay=1000
            },
        ];

        // Test with a current timestamp that allows some delays to pass
        let current_timestamp = base_timestamp + 200; // 1200

        // Delay function: fast=100, medium=500, slow=1000
        let delay_fn = |actor: ActorId| -> u64 {
            if actor == actor_fast {
                100
            } else if actor == actor_medium {
                500
            } else if actor == actor_slow {
                1000
            } else {
                0
            }
        };

        let mut messages = Messages::new(data.len());
        for message in &data {
            assert!(messages.add(message.clone()).is_some());
        }

        let removed: Vec<_> = messages.drain(&root, current_timestamp, delay_fn).collect();

        // Only the first message (actor_fast) should be drained
        // current_timestamp (1200) >= root.timestamp + delay(actor_fast) (1000 + 100 = 1100)
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].source, actor_fast);
        assert_eq!(messages.len(), 2); // Two messages should remain

        // Test with later timestamp that allows more draining
        let later_timestamp = base_timestamp + 600; // 1600
        let removed: Vec<_> = messages.drain(&root, later_timestamp, delay_fn).collect();

        // Now the medium delay message should also be drained
        // current_timestamp (1600) >= root.timestamp + delay(actor_medium) (1000 + 500 = 1500)
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].source, actor_medium);
        assert_eq!(messages.len(), 1); // One message should remain

        // Test with even later timestamp that drains everything
        let much_later_timestamp = base_timestamp + 1100; // 2100
        let removed: Vec<_> = messages
            .drain(&root, much_later_timestamp, delay_fn)
            .collect();

        // Now the slow delay message should also be drained
        // current_timestamp (2100) >= root.timestamp + delay(actor_slow) (1000 + 1000 = 2000)
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].source, actor_slow);
        assert_eq!(messages.len(), 0); // No messages should remain
    }

    #[test]
    fn messages_drain_edge_cases() {
        let actor = ActorId::from([42; 32]);

        let root = RelayedMerkleRoot {
            block: GearBlockNumber(100),
            block_hash: hex!("410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848")
                .into(),
            authority_set_id: AuthoritySetId(1),
            merkle_root: hex!("bfd87951376d18fe27f106b603a4f082d83f0cc4da3c5bebb61ab276cb8033fe")
                .into(),
            timestamp: 1000,
        };

        let message = accumulator::Request {
            block: GearBlockNumber(100),
            block_hash: hex!("38f753a5d02c81e91ff8b3950c2cd03c526ced9abc0b6ef29803ee4250a0df85")
                .into(),
            authority_set_id: AuthoritySetId(1),
            tx_uuid: Uuid::now_v7(),
            source: actor,
        };

        // Test with zero delay
        {
            let mut messages = Messages::new(10);
            messages.add(message.clone()).unwrap();

            let removed: Vec<_> = messages.drain(&root, 1000, |_| 0).collect();
            assert_eq!(removed.len(), 1);
            assert_eq!(messages.len(), 0);
        }

        // Test with current timestamp exactly equal to threshold
        {
            let mut messages = Messages::new(10);
            messages.add(message.clone()).unwrap();

            let removed: Vec<_> = messages.drain(&root, 1500, |_| 500).collect();
            assert_eq!(removed.len(), 1); // Should be drained since 1500 >= 1000 + 500
            assert_eq!(messages.len(), 0);
        }

        // Test with current timestamp one less than threshold
        {
            let mut messages = Messages::new(10);
            messages.add(message.clone()).unwrap();

            let removed: Vec<_> = messages.drain(&root, 1499, |_| 500).collect();
            assert_eq!(removed.len(), 0); // Should NOT be drained since 1499 < 1000 + 500
            assert_eq!(messages.len(), 1);
        }

        // Test with future timestamp (should not be drained)
        {
            let mut messages = Messages::new(10);
            messages.add(message.clone()).unwrap();

            let removed: Vec<_> = messages.drain(&root, 999, |_| 0).collect();
            assert_eq!(removed.len(), 0); // Should NOT be drained since 999 < 1000 + 0
            assert_eq!(messages.len(), 1);
        }
    }

    #[test]
    fn messages_drain_all() {
        let data = [
            // Authority set 1000, block 100
            accumulator::Request {
                block: GearBlockNumber(100),
                block_hash: hex!(
                    "1111111111111111111111111111111111111111111111111111111111111111"
                )
                .into(),
                authority_set_id: AuthoritySetId(1000),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
            // Authority set 1000, block 200
            accumulator::Request {
                block: GearBlockNumber(200),
                block_hash: hex!(
                    "2222222222222222222222222222222222222222222222222222222222222222"
                )
                .into(),
                authority_set_id: AuthoritySetId(1000),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
            // Authority set 1001, block 50
            accumulator::Request {
                block: GearBlockNumber(50),
                block_hash: hex!(
                    "3333333333333333333333333333333333333333333333333333333333333333"
                )
                .into(),
                authority_set_id: AuthoritySetId(1001),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
            // Authority set 1001, block 150
            accumulator::Request {
                block: GearBlockNumber(150),
                block_hash: hex!(
                    "4444444444444444444444444444444444444444444444444444444444444444"
                )
                .into(),
                authority_set_id: AuthoritySetId(1001),
                tx_uuid: Uuid::now_v7(),
                source: ActorId::zero(),
            },
        ];

        let mut messages = Messages::new(data.len());
        for message in &data {
            messages.add(message.clone()).unwrap();
        }

        // Test draining with authority set 1000, block 150
        // The drain_all logic drains messages with the same authority set ID and block <= target block
        let root = RelayedMerkleRoot {
            block: GearBlockNumber(150),
            block_hash: hex!("5555555555555555555555555555555555555555555555555555555555555555")
                .into(),
            authority_set_id: AuthoritySetId(1000),
            merkle_root: hex!("6666666666666666666666666666666666666666666666666666666666666666")
                .into(),
            timestamp: 0,
        };

        let removed: Vec<_> = messages.drain_all(&root).collect();

        // Should drain messages with the same authority set (1000) and block <= 150
        // Based on the compare function and drain_all logic:
        // It drains messages from the start of the authority set to the target block
        assert_eq!(removed.len(), 1); // Only block 100 matches (authority_set_id=1000, block<=150)
        assert_eq!(removed[0].authority_set_id, AuthoritySetId(1000));
        assert_eq!(removed[0].block, GearBlockNumber(100));
        assert_eq!(messages.len(), 3);

        // Test draining with higher authority set and block
        let root2 = RelayedMerkleRoot {
            block: GearBlockNumber(250),
            block_hash: hex!("7777777777777777777777777777777777777777777777777777777777777777")
                .into(),
            authority_set_id: AuthoritySetId(1001),
            merkle_root: hex!("8888888888888888888888888888888888888888888888888888888888888888")
                .into(),
            timestamp: 0,
        };

        let removed2: Vec<_> = messages.drain_all(&root2).collect();

        // This should drain messages with authority_set_id == 1001 and block <= 250
        // Only 2 messages match: both 1001 authority set messages
        assert_eq!(removed2.len(), 2); // Two messages from authority set 1001
        assert_eq!(messages.len(), 1); // One message remains (authority set 1000, block 200)
    }

    #[test]
    fn messages_drain_timestamp_with_merkle_roots() {
        let actor1 = ActorId::from([1; 32]);
        let actor2 = ActorId::from([2; 32]);
        let actor3 = ActorId::from([3; 32]);

        // Create merkle roots with different timestamps
        let root1 = RelayedMerkleRoot {
            block: GearBlockNumber(100),
            block_hash: hex!("1111111111111111111111111111111111111111111111111111111111111111")
                .into(),
            authority_set_id: AuthoritySetId(1000),
            merkle_root: hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .into(),
            timestamp: 1000,
        };

        let root2 = RelayedMerkleRoot {
            block: GearBlockNumber(200),
            block_hash: hex!("2222222222222222222222222222222222222222222222222222222222222222")
                .into(),
            authority_set_id: AuthoritySetId(1001),
            merkle_root: hex!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                .into(),
            timestamp: 2000,
        };

        let mut merkle_roots = MerkleRoots::new(10);
        merkle_roots.add(root1).unwrap();
        merkle_roots.add(root2).unwrap();
        let newer = RelayedMerkleRoot {
            block: GearBlockNumber(110),
            block_hash: [3; 32].into(),
            merkle_root: [3; 32].into(),
            timestamp: 2250,
            ..root1
        };
        merkle_roots.add(newer).unwrap();

        let messages_data = [
            // Message that should match with delay
            accumulator::Request {
                block: GearBlockNumber(50),
                block_hash: hex!(
                    "3333333333333333333333333333333333333333333333333333333333333333"
                )
                .into(),
                authority_set_id: AuthoritySetId(1000),
                tx_uuid: Uuid::from_u128(1),
                source: actor1, // delay=100
            },
            // Message that should match with delay
            accumulator::Request {
                block: GearBlockNumber(150),
                block_hash: hex!(
                    "4444444444444444444444444444444444444444444444444444444444444444"
                )
                .into(),
                authority_set_id: AuthoritySetId(1001),
                tx_uuid: Uuid::from_u128(2),
                source: actor2, // delay=200
            },
            // Message that should not match any root due to high delay
            accumulator::Request {
                block: GearBlockNumber(50),
                block_hash: hex!(
                    "5555555555555555555555555555555555555555555555555555555555555555"
                )
                .into(),
                authority_set_id: AuthoritySetId(1000),
                tx_uuid: Uuid::from_u128(3),
                source: actor3, // delay=2000
            },
        ];

        let delay_fn = |actor: ActorId| -> u64 {
            if actor == actor1 {
                100
            } else if actor == actor2 {
                200
            } else {
                2000
            }
        };

        let mut messages = Messages::new(messages_data.len());
        for message in &messages_data {
            messages.add(message.clone()).unwrap();
        }

        // Test with timestamp that allows first two messages to be drained
        let current_timestamp = 2300; // Should allow matching against roots

        let removed: Vec<_> = messages
            .drain_timestamp(current_timestamp, delay_fn, &merkle_roots)
            .collect();

        // Should drain messages that have appropriate roots found with the delay
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0], (root1, messages_data[0].clone()));
        assert_eq!(removed[1], (root2, messages_data[1].clone()));
        assert_eq!(messages.len(), 1);

        // The remaining message should be the one with high delay
        assert_eq!(messages.0[0].source, actor3);

        let later_message = accumulator::Request {
            block: GearBlockNumber(101),
            tx_uuid: Uuid::from_u128(4),
            ..messages_data[0].clone()
        };
        messages.add(later_message.clone()).unwrap();
        assert!(messages
            .drain_timestamp(2349, delay_fn, &merkle_roots)
            .next()
            .is_none());
        assert_eq!(
            messages
                .drain_timestamp(2350, delay_fn, &merkle_roots)
                .collect::<Vec<_>>(),
            vec![(newer, later_message)]
        );
        assert_eq!(
            messages
                .drain_timestamp(3000, delay_fn, &merkle_roots)
                .collect::<Vec<_>>(),
            vec![(root1, messages_data[2].clone())]
        );
    }
}
