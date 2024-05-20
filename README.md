# Gear bridges

Implementation of zk-based bridge to ethereum for gear-based blockchains.

## High-level gear -> eth design

Gear -> eth protocol allows relaying messages from gear-based blockchains to ethereum. Messages are
some generic data defined by protocols built on top of bridge. Protocol doesn't guarantee order in which messages are relayed.

This repository contains implementation of token bridging protocol built on top of more generic messaging protocol.

##### Components present in one-directional gear -> eth bridge:

![gear -> eth](https://github.com/gear-tech/gear-bridges/blob/main/images/gear_eth.png)

- `GRC-20` - program capable of transferring, burning and minting `GRC-20` tokens.
- `GRC-20 gateway` - receive `GRC-20` tokens from users, burns them and emits message to `pallet-gear-bridge` built-in actor. This message contains information about which token is getting bridged, how much of it and receipient of funds on ethereum network.
- `pallet-gear-bridges` built-in actor - entrypoint into generic bridging protocol. Receives messages from any actor on gear network and relays into `pallet-gear-bridge`.
- `pallet-gear-bridge` - receives messages from `pallet-gear-bridges` built-in actor and stores them in the binary merkle trie. This merkle trie gets slashed at the end of each `era`.
- `backend` - reads gear state, queues zk-proof generation and submits zk-proofs to ethereums.
- `prover` - generally capable of creating 2 types of zk-proofs: proof of authority set changes and proof of inclusion of merkle trie root into the storage of `pallet-gear-bridge`. Combination of these  proofs allows for trustless relaying of merkle trie roots from `pallet-gear-brisge` storage to ethereum.
- `relayer contract` - accept proofs of merkle trie root inclusion and if they're valid then stores merkle trie roots in the memory.
- `gnark-verifier` - contract capable of verifying `plonk` proofs created by [gnark](https://github.com/Consensys/gnark). The submitted proofs are just [plonky2](https://github.com/0xPolygonZero/plonky2) proofs wrapped by `gnark`.
- `message queue contract` - used to recover messages from merkle tries. User can request message to be relayed further onto ethereum by providing proof of inclusion of some message that's actually included into merkle trie and given that this merkle root was already relayed by `backend`(or some other party). Also it's an exit point of generic gear -> eth bridging protocol.
- `ERC20 treasury` - treasury that accept user funds and release them. Release can only be triggered by message relayed over bridge which source is `GRC-20 gateway`.

##### Workflow of gear -> eth token transfer:

![gear -> eth transfer](https://github.com/gear-tech/gear-bridges/blob/main/images/gear_eth_transfer.png)

- user submits message to `GRC-20 gateway` to initiate bridging.
- `GRC20 gateway` burns `GRC-20` tokens and emits message to `pallet-gear-bridge` built-in actor.
- `pallet-gear-bridge` built-in actor relays message to `pallet-gear-bridge`.
- `pallet-gear-bridge` stores message in merkle trie.
- eventually `backend`(or some other party) relays message to `relayer contract` and it gets stored there.
- user see that his message was relayed and submits merkle proof of inclusion to `message queue contract`.
- `message queue contract` reads merkle root from `relayer contract`, checks merkle proof and relays message to `ERC20 treasury`.
- `ERC20 treasury` releases funds to user account on ethereum.

## Prover circuits

### Block finality
Proves that some block was finalized by some authority set on the gear chain.

![block finality circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/block_finality_circuit.png)

### Validator set change
Proves that validator set have changed.Validator set change means that current validator set finalized a block containing next validator set in its storage.

![validator set change circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/next_validator_set_circuit.png)

### Substrate storage trie circuits
There are only two types of nodes currently supported for now:

#### Branch node without value
![branch node parser circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/mpt_branch_node_parser_circuit.png)

#### Hashed value leaf
![leaf node parser circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/mpt_leaf_node_parser_circuit.png)

These proofs are composed into storage proof, which can prove that some block contains data in its storage at particular address:

![storage proof circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/storage_proof_circuit.png)

### Recent validator set
Used to prove chain of validator set changes to prove transition from genesis to the recent validator set in one proof. Genesis validator set is present as a constant in the circuit.

![recent validator set circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/recent_validator_set_circuit.png)

### Message inclusion
Used to prove that specific message merkle root was submitted on gear chain for bridging, that is, included into `pallet-gear-bridge` storage.

![message sent circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/message_sent_circuit.png)

### Final proof
Proof that's submitted to ethereum. Proves that message merkle root was present in storage of `pallet-gear-bridge` at some finalized block.

![final proof circuit](https://github.com/gear-tech/gear-bridges/blob/main/images/final_proof_circuit.png)
