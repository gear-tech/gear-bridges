# Circuits and proofs

The bridge uses recursive Plonky2 circuits to prove Gear finality and storage inclusion, then wraps the resulting proof in a gnark Plonk circuit so Ethereum can verify it with Solidity verifier contracts.

The main Rust implementation is under [prover/src](../prover/src). The Ethereum verifier boundary is [ethereum/src/interfaces/IVerifier.sol](../ethereum/src/interfaces/IVerifier.sol), [ethereum/src/VerifierMainnet.sol](../ethereum/src/VerifierMainnet.sol), and [ethereum/src/VerifierTestnet.sol](../ethereum/src/VerifierTestnet.sol).

## Proof statement

The final proof establishes, at a high level:

1. a Gear block is finalized by the authority set that signed its GRANDPA justification;
2. the finalized block contains the relevant bridge storage item;
3. the storage item contains the message-queue root;
4. the authority-set proof descends from the configured genesis authority set;
5. the public root and Gear block number are the values supplied to Ethereum.

The proof does not prove that a token transfer UI request was made, that a token contract is correctly configured, or that a user has waited through MessageQueue's message delay. Those are separate protocol and token-relayer responsibilities.

## Circuit layers

### Block finality

[prover/src/block_finality/mod.rs](../prover/src/block_finality/mod.rs) parses a GRANDPA pre-commit message and proves that more than two thirds of the authority set signed the finalized block. The circuit:

- hashes the validator public keys with Blake2;
- checks the Ed25519 signatures used by the GRANDPA pre-commits;
- checks the pre-commit discriminant and message fields;
- exposes the authority-set hash and GRANDPA message as public inputs.

The implementation derives the required signer count as (2 * validator_count) / 3 + 1. The prover constants cap the supported validator count at 64. The bridge's core finality proof uses GRANDPA/Ed25519; the presence of the reusable plonky2_ecdsa crate does not mean ECDSA is part of this final proof.

### Storage inclusion

[prover/src/storage_inclusion/mod.rs](../prover/src/storage_inclusion/mod.rs) proves that a storage value is present beneath a Substrate state root. It composes:

- a block-header parser;
- a storage address circuit;
- branch and leaf node parsers;
- Blake2 hashing and trie-root reconstruction.

The circuit currently supports StorageValue-style entries and the node forms documented in the source. It does not generally support StorageMap/StorageDoubleMap layouts, BranchWithValue nodes, or inlined values shorter than 32 bytes. Check the source comments and tests before adding a new storage representation.

For the bridge root, the storage item is the data encoding whose Blake2 hash is connected to the storage-inclusion proof. The raw storage bytes and trie node data come from Gear RPC inclusion-proof DTOs and are converted by [relayer/src/prover_interface.rs](../relayer/src/prover_interface.rs).

### Authority-set recursion

The latest-validator-set circuits recursively link authority-set changes from the configured genesis set to the set that finalized the target block. A transition proves:

- a block finalized by the current set;
- inclusion of the next authority-set data in that block's storage;
- consistency of the next set hash;
- recursion from the previous authority-set proof.

The root relayer stores these intermediate proofs through ProofStorage so every message root does not need to rebuild the entire authority history.

GenesisConfig is part of the circuit statement. Changing the genesis authority-set id or hash changes the proof's fixed inputs and the resulting circuit digest. Existing proof storage and the deployed Solidity verifier must be treated as one compatibility unit.

### Message-sent composition

[prover/src/final_proof/message_sent.rs](../prover/src/final_proof/message_sent.rs) composes:

- a block-finality proof;
- a storage-inclusion proof;
- a chain of Blake2-hashed Gear headers;
- the raw message/root storage bytes.

The header chain connects the block hash in the storage proof to the block hash in the finality proof. The storage bytes are hashed inside the circuit and connected to the storage-inclusion target. This prevents a caller from supplying a root that is unrelated to the proven storage leaf.

### Final proof

[prover/src/final_proof/mod.rs](../prover/src/final_proof/mod.rs) recursively verifies the MessageSent proof and the latest-validator-set proof. It connects:

- the validator-set hash in MessageSent to the current authority-set hash;
- the authority-set id in MessageSent to the current authority-set id;
- the current proof's genesis id/hash to GenesisConfig;
- the message contents/root and block number as the final public inputs.

The public target is therefore the 32-byte message-queue root plus the Gear block number. The relayer extracts those values from the gnark public-input response into [relayer/src/prover_interface.rs](../relayer/src/prover_interface.rs).

## Plonky2 representation

The Rust prover uses the Goldilocks field and Poseidon configuration from [prover/src/lib.rs](../prover/src/lib.rs), with recursion depth parameter D equal to 2. Circuit data contains the common circuit data and verifier-only data required to verify a serialized Plonky2 proof.

The reusable circuit crates include:

- plonky2_blake2b256 for generic Blake2 hashing;
- plonky2_sha512 for SHA-512;
- plonky2_ed25519 for Ed25519 operations;
- plonky2_ecdsa for secp256k1/ECDSA gadgets;
- plonky2_u32 for arithmetic, range checks, comparisons, and witness helpers.

The final bridge path uses only the pieces wired by the prover modules described above. Treat other circuit crates as libraries unless a call path from FinalProof or the relayer proves otherwise.

## Plonky2 to gnark to Solidity

The bridge crosses a deliberate format boundary:

~~~text
Rust Plonky2 proof
       |
       v
ExportedProofWithCircuitData JSON
       |
       v
Rust C ABI -> gnark-wrapper
       |
       v
BN254 Plonk proof + compressed public inputs
       |
       v
Solidity Verifier -> MessageQueue
~~~

The Rust type ExportedProofWithCircuitData contains JSON strings for:

- the Plonky2 proof with public inputs;
- common circuit data;
- verifier-only circuit data.

The Rust FFI adapter in [relayer/src/prover_interface.rs](../relayer/src/prover_interface.rs) serializes that value and calls the exported prove function. [gnark-wrapper/main.go](../gnark-wrapper/main.go) then:

1. loads the proving key and R1CS from the configured data directory;
2. compiles the gnark circuit and creates keys when the proving key is absent;
3. verifies the inner Plonky2 proof inside the gnark circuit;
4. creates a BN254 Plonk proof;
5. verifies that proof locally with the verifying key;
6. serializes the proof with gnark's Solidity format;
7. compresses the Goldilocks public inputs into the two Ethereum public inputs.

The data directory can therefore contain SRS material, R1CS, proving/verifying keys, and a generated verifier.sol. Mount it as persistent, deployment-specific data. Keep proving data separated when running multiple networks whose verifier keys differ.

The generated Solidity libraries [PlonkVerifierMainnet.sol](../ethereum/src/libraries/PlonkVerifierMainnet.sol) and [PlonkVerifierTestnet.sol](../ethereum/src/libraries/PlonkVerifierTestnet.sol) explicitly say they are generated code. Regenerate through the established Go/Rust workflow and review the output; do not hand-edit generated verifier code.

## Ethereum verification boundary

MessageQueue.submitMerkleRoot receives:

- the uint256 Gear block number;
- the bytes32 root;
- the serialized gnark Plonk proof.

It packs the root and the low 32 bits of the block number into two public inputs, then calls the configured IVerifier. A proof can therefore be valid only when:

- the gnark proof matches the deployed verifier's circuit;
- the verifier's fixed circuit digest and constants match the Rust proof;
- the root and block number match the proof public inputs;
- MessageQueue accepts the block-range and challenge/emergency-state checks.

After verification, the contract stores the root by Gear block number. MessageQueue.processMessage performs a separate binary Merkle proof check for an individual message and enforces the root delay.

## Where data comes from

The relayer assembles circuit inputs from Gear RPC:

- GRANDPA justifications and pre-commits for BlockFinality;
- block headers for the header chain;
- storage trie branches/leaves and the original stored data for StorageInclusion;
- the authority-set transition data for recursive authority proofs;
- the queue root and block-inclusion proof for the final proof.

[relayer/src/prover_interface.rs](../relayer/src/prover_interface.rs) converts RPC DTOs into prover-owned structures, reverses branch-node order into root-to-leaf order, expands storage addresses into nibbles, and extracts the final block/root from gnark public inputs.

A proof failure can therefore originate in RPC evidence, trie encoding, authority-set history, circuit compatibility, gnark setup, or Ethereum contract state. The error message alone may not identify the layer; inspect the logs around the proof stage.

## Runtime and memory constraints

The main runtime sets up Rayon and Tokio workers before starting relayers. The root relayer's default thread count is 24, and each proof thread can allocate substantial memory. The prover also builds a thread pool for header hashing and reads RUST_MIN_STACK when composing MessageSent.

For operators:

- set RUST_MIN_STACK to at least 4194304 bytes;
- start with a conservative thread_count on a new host;
- keep SRS/key data on local durable storage;
- do not share filesystem proof/key directories between independent relayers;
- reserve CPU and memory for the gnark proving phase as well as Rust circuits.

For maintainers, treat circuit digest, verifier-only data, gnark R1CS, proving key, verifying key, Solidity verifier, and deployment network as a single versioned artifact set.

## Tests and debugging

Run focused tests from the repository root:

~~~sh
cargo test -p prover
cargo test -p plonky2_blake2b256
cargo test -p plonky2_ecdsa
cargo test -p plonky2_ed25519
cargo test -p plonky2_u32
~~~

The Ethereum circuit/verifier tests live under [ethereum/test](../ethereum/test) and can be run with Foundry when dependencies are installed:

~~~sh
cd ethereum
forge test
~~~

The gnark wrapper has Go tests:

~~~sh
cd gnark-wrapper
go test ./...
~~~

For an integration-oriented relayer check:

~~~sh
cargo check -p relayer
cargo test -p relayer merkle_roots
cargo test -p relayer prover
~~~

When debugging a failed final proof, classify it in this order:

1. Does the Gear RPC return the requested finalized block and a consistent GRANDPA proof?
2. Does the stored data hash match the trie inclusion proof and expected queue root?
3. Does proof storage contain the authority-set proof for the signer set?
4. Does the gnark data directory contain matching SRS/key/verifier material?
5. Does the returned public input encode the expected root and Gear block number?
6. Does the deployed MessageQueue point at the expected verifier and accept the block range?
