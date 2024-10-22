# Prover Circuits

## Block Finality

The Block Finality circuit proves that a specific block was finalized by an authority set on the Gear chain. This involves verifying that a majority (>2/3) of validators have signed the GRANDPA vote for the block.

![block finality circuit](/images/prover/BlockFinality.png)

## Block Storage Inclusion

This circuit can prove that some storage item is present in the substrate's merkle-patricia trie of some block identified by it's hash.

![block storage inclusion circuit](/images/prover/BlockStorageInclusion.png)

Some additional circuits that're used to compose this circuit are:

#### Branch Node Parser

This circuit parses and checks validity of branch nodes from substrate's merkle-patricia trie. For now only nodes of type `Branch Node Without Value` are supported.

![branch node parser circuit](/images/prover/BranchNodeParser.png)

#### Leaf Node Parser

This circuit parses and checks validity of leaf nodes from substrate's merkle-patricia trie. For now only nodes of type `Hashed Value Leaf` are supported.

![leaf node parser circuit](/images/prover/LeafNodeParser.png)

#### Generic Blake2 circuit

This circuit can assert that `blake2` hash is computed correctly. It's a wrapper around `/circuits/plonky2_blake2b256` that can accept inputs of arbitrary length(up to some limit) and still have constant `circuit_digest`.

## Final Proof

The Final Proof can prove that some block with number `N` contains some hash `H` in `pallet-gear-eth-bridge` storage and this block is finalized on gear network and included into chain(correctness is proven based on invariants existing on consensus layer of gear protocol). This proof is verified inside of `gnark` circuit (see `/gnark-wrapper/main.go`) and submitted to ethereum.

![final proof circuit](/images/prover/FinalProof.png)

Some additional circuits that're used to compose this circuit are:

#### Authority Set Rotation

The Authority Set Rotation circuit proves that the validator set has changed. This change means that the current validator set finalized a block containing the next validator set hash in the storage of `pallet-gear-eth-bridge`.

#### Latest Authority Set

The Latest Authority Set circuit is used to prove a chain of authority set changes, demonstrating the transition from the genesis authority set to the most recent one. The genesis authority set means some arbitrarily-selected authority set and stored as a constant within the circuit. When circuits are built it's parsed from CLI args.

#### Messages Trie Root Inclusion Into Storage

Messages are stored as a binary merkle trie root. This merkle trie accumulates all the messages sent to `pallet-gear-eth-bridge` from the start of a current era (~12 hours on gear networks). Messages Trie Root Inclusion Into Storage circuit can prove that such a merkle root is included into the storage of some block defined by it's hash.
