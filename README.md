# Gear bridges

Implementation of zk-based bridge to ethereum for gear-based blockchains.

## Circuits

### block finality
Proves that some block was correctly finalized on the gear chain.

![block finality circuit](https://github.com/mertwole/gear-bridges/blob/main/images/block_finality_circuit.png)

### validator set change
Used to compose `recent validator set` proof.

![validator set change circuit](https://github.com/mertwole/gear-bridges/blob/main/images/next_validator_set_circuit.png)

### substrate storage trie circuit
There are only two types of nodes currently supported for now:

#### branch node without value
![branch node parser circuit](https://github.com/mertwole/gear-bridges/blob/main/images/mpt_branch_node_parser_circuit.png)

#### hashed value leaf
![leaf node parser circuit](https://github.com/mertwole/gear-bridges/blob/main/images/mpt_leaf_node_parser_circuit.png)

They're composed into storage proof, which prove that some block contains data in storage at particular address

![storage proof circuit](https://github.com/mertwole/gear-bridges/blob/main/images/storage_proof_circuit.png)

### recent validator set
Used to prove chain of validator set changes to prove transition from genesis to the recent validator set in one proof.

![recent validator set circuit](https://github.com/mertwole/gear-bridges/blob/main/images/recent_validator_set_circuit.png)

### message sent
Used to prove that specific message was submitted on gear chain for bridging.

![message sent circuit](https://github.com/mertwole/gear-bridges/blob/main/images/message_sent_circuit.png)

### final proof
Proof that's submitted to ethereum.

![final proof circuit](https://github.com/mertwole/gear-bridges/blob/main/images/final_proof_circuit.png)
