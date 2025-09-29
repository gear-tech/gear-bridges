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
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Added {
    /// Provided instance is new and added.
    Ok,
    /// The same as Ok but returns the popped oldest merkle root (to
    /// retain the initial capacity).
    Removed(RelayedMerkleRoot),
    /// The provided root overwrites existing one with the same authority set id.
    Overwritten(GearBlockNumber),
}

#[derive(Serialize, Deserialize)]
pub struct MerkleRoots(Vec<RelayedMerkleRoot>);

impl MerkleRoots {
    pub fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    fn compare(
        authority_set_id: AuthoritySetId,
        block_number: GearBlockNumber,
        authority_set_id_new: AuthoritySetId,
        block_number_new: GearBlockNumber,
    ) -> Ordering {
        if authority_set_id_new == authority_set_id {
            return block_number_new.cmp(&block_number);
        }

        authority_set_id_new.cmp(&authority_set_id)
    }

    pub fn find(
        &self,
        authority_set_id: AuthoritySetId,
        block_number: GearBlockNumber,
        last_timestamp: u64,
        delay: u64,
    ) -> Option<&RelayedMerkleRoot> {
        let i = match self.0.binary_search_by(|root| {
            Self::compare(
                root.authority_set_id,
                root.block,
                authority_set_id,
                block_number,
            )
        }) {
            Ok(i) => {
                return self
                    .0
                    .get(i)
                    .filter(|root| last_timestamp >= root.timestamp + delay)
            }

            Err(i) => {
                if i == 0 {
                    return None;
                } else {
                    i
                }
            }
        };

        let result = self.0.get(i - 1)?;
        if result.authority_set_id != authority_set_id || result.timestamp + delay > last_timestamp
        {
            if result.authority_set_id != authority_set_id {
                log::trace!(
                    "Authority set id mismatch: expected {authority_set_id:?}, found {result:?}"
                );
            }

            if result.timestamp + delay > last_timestamp {
                log::trace!(
                    "Timestamp + delay mismatch: last_timestamp = {last_timestamp}, root.timestamp + delay = {} + {} = {}",
                    result.timestamp,
                    delay,
                    result.timestamp + delay
                );
            }
            return None;
        }

        Some(result)
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[allow(dead_code)]
    pub fn get(&self, i: usize) -> Option<&RelayedMerkleRoot> {
        self.0.get(i)
    }

    // Err(i) -> there is already a root with the same authority_set_id
    pub fn add(&mut self, root_new: RelayedMerkleRoot) -> Result<Added, usize> {
        let i = match self.0.binary_search_by(|root| {
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

        if i != 0 {
            if let Some(root) = self.0.get(i - 1) {
                if root.authority_set_id == root_new.authority_set_id {
                    return Err(i - 1);
                }
            }
        }

        if let Some(root_previous) = self.0.get_mut(i) {
            if root_previous.authority_set_id == root_new.authority_set_id {
                let block_number = root_previous.block;
                *root_previous = root_new;

                return Ok(Added::Overwritten(block_number));
            }
        }

        let (result, i) = if self.0.len() < self.0.capacity() {
            (None, i)
        } else {
            // adjust insertion index
            let i = if i >= self.0.len() { i - 1 } else { i };

            (self.0.pop(), i)
        };

        self.0.insert(i, root_new);

        Ok(match result {
            Some(root) => Added::Removed(root),
            None => Added::Ok,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    #[test]
    fn merkle_roots() {
        const CAPACITY: usize = 2;

        let the_newest_root = RelayedMerkleRoot {
            block: GearBlockNumber(18_686_058),
            block_hash: hex!("d52749e67e5e3fae9a4769330af6587dc96465d70af51c85d4706336aab634e5")
                .into(),
            authority_set_id: AuthoritySetId(1_309),
            merkle_root: hex!("a5c50de3b48386f4159d24f735c067bb2e6f80c0eb3f3ffe862e0aedc19f6e0f")
                .into(),
            timestamp: 0,
        };
        let the_oldest_root = RelayedMerkleRoot {
            block: GearBlockNumber(16_881_711),
            block_hash: hex!("9d75d1c32eac1ea29739e766827708075198c58a2b558a1db5dba78e851bc70f")
                .into(),
            authority_set_id: AuthoritySetId(1_183),
            merkle_root: hex!("8c116ce8293b795eb8dc526d0f614dc745e1b98ef3ca28c75991ea7eb8b127c0")
                .into(),
            timestamp: 0,
        };
        let data = [
            RelayedMerkleRoot {
                block: GearBlockNumber(18_676_002),
                block_hash: hex!(
                    "8d6286038e2ac0bea811e9d99d821084f0271a59b621b4eef52cd85b2fd6c3cb"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_308),
                merkle_root: hex!(
                    "00c39a437f0331e49a996433f95ca3955a9caf77b8bf6a1f10b2d5214326bd91"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_172),
                block_hash: hex!(
                    "38f753a5d02c81e91ff8b3950c2cd03c526ced9abc0b6ef29803ee4250a0df85"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "ca54d4db8284ab35e915723e8efc0303d0ca009892b17552afeea9f92b306c9a"
                )
                .into(),
                timestamp: 0,
            },
            the_oldest_root,
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_289),
                block_hash: hex!(
                    "74dcef50f0cf4299a0774b147a748f3d5961d913afb9d0e74868a298255edea2"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "ae7c9f468b4d0625a4780389f12f0f19637901376f81f37a3394e9b4f81c95fb"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_881_824),
                block_hash: hex!(
                    "24b437d833fc6b7e9aea9d987ca1411ae293d340427da57dd7d9888fda8b16a2"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "8f3dbd805121a1c9f820884f1f5e71c5c9deb733f1238366043b052dd468e390"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_636),
                block_hash: hex!(
                    "410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "bfd87951376d18fe27f106b603a4f082d83f0cc4da3c5bebb61ab276cb8033fe"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_738),
                block_hash: hex!(
                    "f621ed5ddb70acd610bef203e2cebd36fa833ece209612237e3932a7e0852c70"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "3a9db39779ce7a5c741db7d16409267beefb41fdc2e00c9d0a826a80fefa9070"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_884_116),
                block_hash: hex!(
                    "0217f936c40c5c81998220e98d58feb7a2e2fb3cc8afd153d057bb19be3892d8"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "71de089ec319f714cb0e7031745eda5fdfce76170176eeeb99f7b35e1c96fc86"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_884_218),
                block_hash: hex!(
                    "fdf8f319a446bd3a059b6f260ad73aaae161ad8fd252c1acc7ba6ff85784351f"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "1636e359c8f975261880b14abaeb511626bfc80e4cab8446448b12d6ce8275b6"
                )
                .into(),
                timestamp: 0,
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_888_714),
                block_hash: hex!(
                    "b592a0ec4212c81eccee43cfdce35de08ddd705361dc01c557a615ebd74200a0"
                )
                .into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!(
                    "1636e359c8f975261880b14abaeb511626bfc80e4cab8446448b12d6ce8275b6"
                )
                .into(),
                timestamp: 0,
            },
        ];

        assert!(!data.is_sorted_by(|a, b| MerkleRoots::compare(
            a.authority_set_id,
            a.block,
            b.authority_set_id,
            b.block,
        ) == Ordering::Less));

        let mut merkle_roots = MerkleRoots::new(CAPACITY);

        for (i, root) in data.iter().enumerate() {
            let result = merkle_roots.add(*root);

            if i < 2 {
                assert!(matches!(result, Ok(Added::Ok)));
            } else if i == 2 || i == 4 {
                assert!(result.is_err());
            } else {
                assert!(matches!(result, Ok(Added::Overwritten(_))));
            }
        }

        assert!(merkle_roots.0.is_sorted_by(|a, b| MerkleRoots::compare(
            a.authority_set_id,
            a.block,
            b.authority_set_id,
            b.block,
        ) == Ordering::Less));
        assert!(
            matches!(merkle_roots.get(merkle_roots.len() - 1), Some(root) if root == data.last().unwrap())
        );

        // searching for contained roots should be successful
        for i in 0..merkle_roots.len() {
            let root = merkle_roots.get(i).unwrap();
            assert!(
                matches!(merkle_roots.find(root.authority_set_id, root.block, 0, 0), Some(result) if root == result)
            );
        }

        // attempt to add a merkle root with the same authority set id and block number
        // should fail
        let result = merkle_roots.add(*merkle_roots.get(0).unwrap());
        assert!(matches!(result, Err(0)));

        let result = merkle_roots
            .find(
                the_oldest_root.authority_set_id,
                the_oldest_root.block,
                0,
                0,
            )
            .unwrap();
        let last = merkle_roots.get(merkle_roots.len() - 1).unwrap();
        assert_eq!(last, result, "last = {last:?}, result = {result:?}");
        assert_ne!(
            last, &the_oldest_root,
            "last = {last:?}, the_oldest_root = {the_oldest_root:?}"
        );

        assert!(merkle_roots
            .find(
                AuthoritySetId(the_oldest_root.authority_set_id.0 - 1),
                GearBlockNumber(0),
                0,
                0
            )
            .is_none());
        assert!(merkle_roots
            .find(
                AuthoritySetId(the_newest_root.authority_set_id.0),
                GearBlockNumber(0),
                0,
                0
            )
            .is_none());

        // attempt to add a newer merkle root should displace the oldest one
        let root_expected_removed = *merkle_roots.get(merkle_roots.len() - 1).unwrap();
        let result = merkle_roots.add(the_newest_root);
        let Added::Removed(root_removed) = result.unwrap() else {
            unreachable!();
        };
        assert_eq!(root_expected_removed, root_removed);
        assert!(matches!(merkle_roots.get(0), Some(root) if root == &the_newest_root));
        assert_eq!(merkle_roots.0.capacity(), CAPACITY);
        assert_eq!(merkle_roots.0.len(), CAPACITY);

        // searching for contained roots should be successful
        for i in 0..merkle_roots.len() {
            let root = merkle_roots.get(i).unwrap();
            assert!(
                matches!(merkle_roots.find(root.authority_set_id, root.block, 0, 0), Some(result) if root == result)
            );
        }

        // request to find with a lesser block number should be responded with a next merkle root
        let result = merkle_roots.find(
            the_newest_root.authority_set_id,
            GearBlockNumber(the_newest_root.block.0 - 1),
            0,
            0,
        );
        assert!(
            matches!(result, Some(root) if root == &the_newest_root),
            "result = {result:?}, the_newest_root = {the_newest_root:?}"
        );

        assert!(merkle_roots
            .find(
                the_newest_root.authority_set_id,
                GearBlockNumber(the_newest_root.block.0 + 1),
                0,
                0
            )
            .is_none());
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

        let messages_data = [
            // Message that should match with delay
            accumulator::Request {
                block: GearBlockNumber(50),
                block_hash: hex!(
                    "3333333333333333333333333333333333333333333333333333333333333333"
                )
                .into(),
                authority_set_id: AuthoritySetId(1000),
                tx_uuid: Uuid::now_v7(),
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
                tx_uuid: Uuid::now_v7(),
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
                tx_uuid: Uuid::now_v7(),
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
        assert_eq!(messages.len(), 1);

        // The remaining message should be the one with high delay
        assert_eq!(messages.0[0].source, actor3);
    }
}
