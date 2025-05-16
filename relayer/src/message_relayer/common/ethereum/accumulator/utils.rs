use super::*;

pub struct Messages(Vec<MessageInBlock>);

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

    // None -> the inner vector is full so the message is rejected
    pub fn add(&mut self, message_new: MessageInBlock) -> Option<()> {
        if self.0.len() >= self.0.capacity() {
            return None;
        }

        match self.0.binary_search_by(|message| Self::compare(
            message.authority_set_id,
            message.block,
            message_new.authority_set_id,
            message_new.block,
        ))
        {
            Ok(i) | Err(i) => self.0.insert(i, message_new),
        }

        Some(())
    }

    pub fn drain(&mut self, merkle_root: &RelayedMerkleRoot) -> Drain<'_, MessageInBlock> {
        let index_end = match self.0.binary_search_by(|message| Self::compare(
            message.authority_set_id,
            message.block,
            merkle_root.authority_set_id,
            merkle_root.block,
        ))
        {
            Ok(i) => i + 1,
            Err(i) => i,
        };

        let index_start = match self.0.binary_search_by(|message| Self::compare(
            message.authority_set_id,
            message.block,
            merkle_root.authority_set_id,
            GearBlockNumber(0),
        ))
        {
            Ok(i) | Err(i) => i,
        };

        self.0.drain(index_start..index_end)
    }
}

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

    pub fn find(&self, authority_set_id: AuthoritySetId, block_number: GearBlockNumber) -> Option<&RelayedMerkleRoot> {
        let i = match self.0.binary_search_by(|root| Self::compare(
            root.authority_set_id,
            root.block,
            authority_set_id,
            block_number,
        ))
        {
            Ok(i) => return self.0.get(i),

            Err(i) => if i == 0 {
                return None;
            } else {
                i
            }
        };

        let result = self.0.get(i - 1)?;
        if result.authority_set_id != authority_set_id {
            return None;
        }

        Some(result)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, i: usize) -> Option<&RelayedMerkleRoot> {
        self.0.get(i)
    }

    // Err(i) -> there is already a root with the same authority_set_id and block_number
    // Ok(None) -> provided instance is new and added
    // Ok(Some(_)) -> the same as with None but returns the popped oldest merkle root (to
    // retain the initial capacity)
    pub fn add(&mut self, root_new: RelayedMerkleRoot) -> Result<Option<RelayedMerkleRoot>, usize> {
        let i = match self.0.binary_search_by(|root| Self::compare(
            root.authority_set_id,
            root.block,
            root_new.authority_set_id,
            root_new.block,
        ))
        {
            Ok(i) => return Err(i),

            Err(i) => i,
        };

        let result = if self.0.len() < self.0.capacity() {
            None
        } else {
            self.0.pop()
        };

        self.0.insert(i, root_new);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    #[test]
    fn merkle_roots() {
        let the_newest_root = RelayedMerkleRoot {
                block: GearBlockNumber(16_888_714),
                block_hash: hex!("b592a0ec4212c81eccee43cfdce35de08ddd705361dc01c557a615ebd74200a0").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("1636e359c8f975261880b14abaeb511626bfc80e4cab8446448b12d6ce8275b6").into(),
            };
        let the_oldest_root = 
            RelayedMerkleRoot {
                block: GearBlockNumber(16_881_711),
                block_hash: hex!("9d75d1c32eac1ea29739e766827708075198c58a2b558a1db5dba78e851bc70f").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("8c116ce8293b795eb8dc526d0f614dc745e1b98ef3ca28c75991ea7eb8b127c0").into(),
            };
        let data = [
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_172),
                block_hash: hex!("38f753a5d02c81e91ff8b3950c2cd03c526ced9abc0b6ef29803ee4250a0df85").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("ca54d4db8284ab35e915723e8efc0303d0ca009892b17552afeea9f92b306c9a").into(),
            },
            the_oldest_root.clone(),
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_289),
                block_hash: hex!("74dcef50f0cf4299a0774b147a748f3d5961d913afb9d0e74868a298255edea2").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("ae7c9f468b4d0625a4780389f12f0f19637901376f81f37a3394e9b4f81c95fb").into(),
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_881_824),
                block_hash: hex!("24b437d833fc6b7e9aea9d987ca1411ae293d340427da57dd7d9888fda8b16a2").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("8f3dbd805121a1c9f820884f1f5e71c5c9deb733f1238366043b052dd468e390").into(),
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_636),
                block_hash: hex!("410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("bfd87951376d18fe27f106b603a4f082d83f0cc4da3c5bebb61ab276cb8033fe").into(),
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_883_738),
                block_hash: hex!("f621ed5ddb70acd610bef203e2cebd36fa833ece209612237e3932a7e0852c70").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("3a9db39779ce7a5c741db7d16409267beefb41fdc2e00c9d0a826a80fefa9070").into(),
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_884_116),
                block_hash: hex!("0217f936c40c5c81998220e98d58feb7a2e2fb3cc8afd153d057bb19be3892d8").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("71de089ec319f714cb0e7031745eda5fdfce76170176eeeb99f7b35e1c96fc86").into(),
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(16_884_218),
                block_hash: hex!("fdf8f319a446bd3a059b6f260ad73aaae161ad8fd252c1acc7ba6ff85784351f").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("1636e359c8f975261880b14abaeb511626bfc80e4cab8446448b12d6ce8275b6").into(),
            },
        ];

        assert!(!data.is_sorted_by(|a, b| MerkleRoots::compare(
            a.authority_set_id,
            a.block,
            b.authority_set_id,
            b.block,
        ) == Ordering::Less));

        let mut merkle_roots = MerkleRoots::new(data.len());

        for root in data {
            let result = merkle_roots.add(root);
            assert!(matches!(result, Ok(None)));
        }

        assert!(merkle_roots.0.is_sorted_by(|a, b| MerkleRoots::compare(
            a.authority_set_id,
            a.block,
            b.authority_set_id,
            b.block,
        ) == Ordering::Less));
        assert!(matches!(merkle_roots.get(merkle_roots.len() - 1), Some(root) if root == &the_oldest_root));

        // attempt to add a merkle root with the same authority set id and block number
        // should fail
        let result = merkle_roots.add(*data.last().unwrap());
        assert!(matches!(result, Err(0)));

        // attempt to add a newer merkle root should displace the oldest one
        let result = merkle_roots.add(the_newest_root);
        assert!(matches!(result, Ok(Some(root)) if root == the_oldest_root));
        assert!(matches!(merkle_roots.get(0), Some(root) if root == &the_newest_root));
        assert_eq!(merkle_roots.0.capacity(), data.len());
        assert_eq!(merkle_roots.0.len(), data.len());

        for i in 0..merkle_roots.len() {
            let root = merkle_roots.get(i).unwrap();
            assert!(matches!(merkle_roots.find(root.authority_set_id, root.block), Some(result) if root == result));
        }

        // request to find with a lesser block number should be responded with a next merkle root
        let result = merkle_roots.find(the_newest_root.authority_set_id, GearBlockNumber(the_newest_root.block.0 - 1));
        assert!(matches!(result, Some(root) if root == &the_newest_root), "result = {result:?}, the_newest_root = {the_newest_root:?}");

        assert!(merkle_roots.find(the_newest_root.authority_set_id, GearBlockNumber(the_newest_root.block.0 + 1)).is_none());

        let result = merkle_roots.find(the_oldest_root.authority_set_id, the_oldest_root.block).unwrap();
        let last = merkle_roots.get(merkle_roots.len() - 1).unwrap();
        assert_eq!(last, result, "last = {last:?}, result = {result:?}");
        assert_ne!(last, &the_oldest_root, "last = {last:?}, the_oldest_root = {the_oldest_root:?}");

        assert!(merkle_roots.find(AuthoritySetId(the_oldest_root.authority_set_id.0 - 1), GearBlockNumber(0)).is_none());
        assert!(merkle_roots.find(AuthoritySetId(the_newest_root.authority_set_id.0 + 1), GearBlockNumber(0)).is_none());
    }

    #[test]
    fn messages() {
        let root = RelayedMerkleRoot {
                block: GearBlockNumber(16_881_826),
                block_hash: hex!("410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848").into(),
                authority_set_id: AuthoritySetId(1_183),
                merkle_root: hex!("bfd87951376d18fe27f106b603a4f082d83f0cc4da3c5bebb61ab276cb8033fe").into(),
            };
        let data = [
            MessageInBlock {
                block: GearBlockNumber(16_883_172),
                block_hash: hex!("38f753a5d02c81e91ff8b3950c2cd03c526ced9abc0b6ef29803ee4250a0df85").into(),
                authority_set_id: AuthoritySetId(1_180),
                message: Default::default(),
            },
            MessageInBlock {
                block: GearBlockNumber(16_883_289),
                block_hash: hex!("74dcef50f0cf4299a0774b147a748f3d5961d913afb9d0e74868a298255edea2").into(),
                authority_set_id: AuthoritySetId(1_183),
                message: Default::default(),
            },
            MessageInBlock {
                block: GearBlockNumber(16_881_824),
                block_hash: hex!("24b437d833fc6b7e9aea9d987ca1411ae293d340427da57dd7d9888fda8b16a2").into(),
                authority_set_id: AuthoritySetId(1_183),
                message: Default::default(),
            },
            MessageInBlock {
                block: GearBlockNumber(16_883_636),
                block_hash: hex!("410d4f4a053a00a32b3655350aa8bde8d458ff0d271ff9927a79fe0f7620f848").into(),
                authority_set_id: AuthoritySetId(1_184),
                message: Default::default(),
            },
        ];

        let mut messages = Messages::new(data.len());
        assert!(messages.drain(&root).collect::<Vec<_>>().is_empty());

        assert!(messages.add(data.get(0).unwrap().clone()).is_some());
        assert!(messages.add(data.get(3).unwrap().clone()).is_some());
        assert!(messages.drain(&root).collect::<Vec<_>>().is_empty());

        let mut messages = Messages::new(data.len());
        for message in &data {
            assert!(messages.add(message.clone()).is_some());
        }

        assert!(messages.add(data[0].clone()).is_none());

        let mut removed = messages.drain(&root).collect::<Vec<_>>();
        let removed_message = removed.pop();
        assert!(matches!(removed_message, Some(ref message) if removed.is_empty() && message == &data[2]), "removed = {removed:?}, removed_message = {removed_message:?}, data[2] = {:?}", data[2]);
        assert_eq!(messages.0.len(), data.len() - 1);
    }
}
