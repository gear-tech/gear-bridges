# Merkle roots

A Gear message-queue root is the commitment that lets Ethereum verify a specific Gear message without trusting the relayer. The root relayer proves that a root was stored in the Gear bridge pallet at a finalized block, then submits the proof to Ethereum's MessageQueue contract.

This page covers the root pipeline, local state machine, batching, persistence, and recovery. For the circuit statements behind the proof, see [circuits and proofs](circuits-and-proofs.md).

## What a root represents

The Gear bridge pallet accumulates queued messages in a binary Merkle trie. A finalized Gear block can emit QueueMerkleRootChanged with a queue id and a root hash. The relayer associates that root with:

- the Gear block number and hash;
- the message nonces observed in the block;
- a raw block-storage inclusion/finality proof;
- the authority-set id that signed the block;
- the local proof/submission state.

The root is not the same thing as an individual message Merkle proof. The root proof establishes that the commitment belongs to a finalized Gear block. Later, a token relayer supplies a separate message inclusion proof to MessageQueue.processMessage.

The relevant on-chain interfaces are [IMessageQueue.sol](../ethereum/src/interfaces/IMessageQueue.sol) and [MessageQueue.sol](../ethereum/src/MessageQueue.sol). The Gear-side extraction and persistence logic is in [relayer/src/merkle_roots/storage.rs](../relayer/src/merkle_roots/storage.rs).

## Source and accumulation

The Gear finalized-block listener receives GRANDPA justifications and stores each block before sending it to consumers. For each block, MerkleRootStorage extracts:

- QueueMerkleRootChanged, as the (queue_id, root) pair;
- MessageQueued nonces, used for priority/spike accounting;
- authority-set changes;
- a raw inclusion proof produced from the block's GRANDPA justification.

A persisted block contains the block hash, an optional root-change record, an authority-set-change flag, and the raw inclusion proof. It is removed from the unprocessed block set only after the corresponding root and authority-set work has been handled. The storage implementation keeps a recent tail of processed blocks while retaining unresolved blocks for recovery.

The Ethereum-side accumulator under [relayer/src/message_relayer/common/ethereum/accumulator/](../relayer/src/message_relayer/common/ethereum/accumulator/) is used by the Gear-to-Ethereum token relayers to track Ethereum roots and message indexes. It should not be confused with the Gear queue-root state machine: the core root relayer submits roots, while the token relayer consumes them.

## Root state machine

Each local root moves through explicit states in [relayer/src/merkle_roots/mod.rs](../relayer/src/merkle_roots/mod.rs):

~~~text
WaitForAuthoritySetSync
          |
          v
    GenerateProof
          |
          v
     SubmitProof
          |
          v
       Finalized

Any stage can become Failed with an error string.
~~~

### WaitForAuthoritySetSync

The final proof needs an inner proof that the authority set signing the block descends from the configured genesis authority set. If proof storage does not contain that authority-set proof, the root is parked and the required block is sent to AuthoritySetSync.

AuthoritySetSync serializes heavy authority-set proving, selects requests by priority, and returns an AuthoritySetSynced response when the proof is available. The root relayer then resubmits waiting blocks for proof generation.

GenesisConfig is a compatibility boundary: its authority-set id and hash are fixed inputs to the recursive proof chain. They must match the deployed verifier and the history from which proof storage was initialized.

### GenerateProof

The root relayer obtains the root (or forces a fresh root read for recovery), determines the signing authority-set id, gathers message nonces and block inclusion material, and sends a request to FinalityProver.

The root is written to local storage before work is handed to the prover. This prevents a restart from losing a proof request that was accepted by the scheduler but not yet completed.

### SubmitProof

When the prover returns, the root stores the serialized final proof and sends it to MerkleRootSubmitter. The submitter tracks the Ethereum transaction separately from the root status, so a process crash can be reconciled against finalized Ethereum state.

### Finalized or Failed

A root becomes Finalized only after the submitter observes the expected root in finalized Ethereum state after the configured confirmation policy. A transaction receipt alone is not enough: the submitter checks that the finalized on-chain root matches the expected value.

Failures are retained in root state and logged. Do not delete the state file immediately; first determine whether the failure is a bad proof/configuration, an Ethereum contract rejection, a transport error, or an exhausted retry policy.

## Authority-set proof chain

Authority-set proofs are stored through the ProofStorage abstraction. The implementation supports:

- filesystem proof storage, where proof files live under a configured directory;
- Gear proof storage, where proof state is maintained through the Gear proof-storage program and a configured directory/cursor.

The first proof starts at the configured GenesisConfig. Subsequent proofs recursively prove authority-set transitions. The genesis authority-set id and 32-byte hash are circuit constants, so using a different genesis configuration with existing proof storage is not a harmless configuration edit.

A missing authority-set proof is normal during catch-up; an authority-set proof that should exist but cannot be loaded during recovery is a storage integrity problem. The startup recovery path can reinstate roots in WaitForAuthoritySetSync, GenerateProof, or SubmitProof, but it cannot reconstruct a missing inner proof without the required Gear history and proof-storage inputs.

## Batching, priority, and forced proofs

The root relayer has three independent ways to request a proof:

1. normal finalized-block processing;
2. priority/recovery processing;
3. the periodic supervisor.

Normal roots are placed in a batch. The batch is flushed when:

- the ordinary spike_timeout elapses;
- a priority request is present and priority_spike_timeout elapses;
- the sum of message nonces in the window reaches spike_threshold.

The spike_window limits how far back timestamps are counted. A bridging-payment event associated with the configured payment address marks the relevant root as priority.

A non-batched proof is used for:

- startup/catch-up recovery;
- a critical block-distance threshold;
- an authority-set-change threshold;
- an authenticated HTTP request for a particular block;
- the periodic supervisor's mismatch check.

The configuration parser accepts a human duration for critical_threshold, but the current effective options convert a timeout to a block-distance using seconds divided by three; the runtime compares that value against finalized Gear block numbers. Treat the setting as a deployment policy and verify it against the exact binary version before changing it.

The startup_sync_strategy setting accepts critical-threshold, skip, or blocks, and the blocks list is validated to be present only for the blocks strategy. The option is carried into the root-relayer configuration; confirm the behavior of the deployed revision when using a non-default strategy.

## Supervisor and on-chain reconciliation

The root relayer periodically reads:

- the latest finalized Gear block and its queue root;
- the root recorded by Ethereum's MessageQueue for the same Gear block;
- local submission state.

If Ethereum has no matching root, or the critical threshold is reached, the supervisor schedules a forced proof. If Ethereum already has the expected root, local storage is marked submitted/confirmed. If local state says a root was submitted but Ethereum does not show it in finalized state, the root remains eligible for recovery.

The supervisor deduplicates a root while the same hash is already in GenerateProof or SubmitProof. It clears that deduplication marker after the submitter reports success or failure.

## Ethereum submission semantics

The submitter calls MessageQueue.submitMerkleRoot with the Gear block number, root hash, and gnark/Plonk proof. The contract:

1. checks genesis and maximum block-distance bounds;
2. derives two packed public inputs from the root and uint32 block number;
3. verifies the Plonk proof through the configured verifier;
4. stores the root and timestamp if the block has not already been assigned a root;
5. emits a MerkleRoot event.

The contract documents that anyone may submit a valid root; the configured fee payer is the process's transaction sender, not a trust assumption for proof validity. MessageQueue can reject submissions during challenge/emergency-stop conditions, and a conflicting root can enable the emergency stop path. Operators must treat such rejections as protocol incidents, not as generic RPC retries.

Once a root is stored, MessageQueue.processMessage still enforces the applicable delay and verifies an individual binary Merkle proof for the message. A root being Finalized in the relayer does not by itself complete a token transfer.

## Persistent format and atomic writes

The root storage path is a JSON file configured as storage.block_storage. It contains a version, unprocessed block records, Ethereum submission states, and a roots map whose keys encode the Gear block number and root hash.

Writes use:

- a temporary file;
- fsync of written contents;
- a backup copy with a .bak extension;
- an atomic rename of the temporary file to the primary path.

On load, a corrupt primary is replaced from the backup when possible. If both files are corrupt, startup fails. Keep the primary and backup on the same durable filesystem and include both in backups.

The root's serialized proof is not the only required artifact. Recovery may also need:

- raw Gear blocks and inclusion proofs;
- authority-set proofs;
- gnark/SRS/key material;
- Ethereum transaction storage used by token relayers;
- the exact genesis and contract configuration.

## Process isolation

Run separate process instances when relayers need different networks, signers, storage, or restart policies. Give each instance its own HTTP address, root-storage file, proof-storage directory, transaction-storage directory, and metrics identity. Sharing mutable state between two processes can cause duplicate work or make recovery ambiguous.

## HTTP proof API

The core relayer enables POST /get_merkle_root_proof on its authenticated server. The request body is:

~~~json
{"blocks":[12345,12346]}
~~~

Send the per-relayer token in X-Token:

~~~sh
curl -fsS \
  -H 'X-Token: replace-me' \
  -H 'Content-Type: application/json' \
  --data '{"blocks":[12345]}' \
  http://127.0.0.1:8443/get_merkle_root_proof
~~~

The response is a JSON array of objects represented by MerkleRootsResponse:

- MerkleRootProof includes the serialized proof bytes, proof block number, requested/root block number, block hash, and root hash.
- NoMerkleRootOnBlock means no usable root was found for the requested block.
- Failed contains an error message.

A request may trigger priority proof generation and wait for completion. Do not expose this endpoint publicly; it can consume proving capacity.

## Operator checks

For a root that is stuck:

1. Check the relayer log for its state: authority-set sync, proof generation, submitter, or Ethereum confirmation.
2. Check root metrics such as last confirmed block, pending roots, batch delay, waiting-for-authority-set count, and pending submissions.
3. Confirm the Gear block and root exist on the finalized chain.
4. Confirm the required authority-set proof exists in the configured proof storage.
5. Confirm gnark data and SRS/key files match the deployed verifier.
6. Check the Ethereum MessageQueue root and transaction status from a finalized RPC view.
7. Preserve the state file and backup before restarting or repairing storage.

Use [usage and operations](usage-and-operations.md) for recovery commands and [running the bridge](running-the-bridge.md) for deployment-level restart guidance.
