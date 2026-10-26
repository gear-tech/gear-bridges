# Gear Bridge documentation

This directory documents the bridge as it is implemented in this repository: outbound BEEFY commitments and Merkle Mountain Range (MMR) inclusion, with independent inbound Ethereum Beacon light-client and receipt authentication. The root [README](../README.md) remains the short project overview; these pages explain how to run, operate, and reason about both authentication and delivery pipelines.

## Start here

- [Running the bridge](running-the-bridge.md): build prerequisites, configuration, Docker, startup, and shutdown.
- [Runtime upgrade and BEEFY activation](runtime-upgrade-and-beefy-activation.md): unsigned governance procedure, real-key readiness, atomic domain binding, and finalized-state checks; preparation is not execution authorization.
- [Usage and operations](usage-and-operations.md): relayer modes, token workflows, manual operations, monitoring, and recovery.
- [Internals](internals.md): component boundaries and end-to-end message flow.
- [Merkle roots](merkle-roots.md): BEEFY/MMR queue-root authentication, publication, persistence, and retained message delivery.
- [Proofs and authentication](circuits-and-proofs.md): outbound BEEFY/MMR and independent inbound Beacon/receipt verification boundaries; the filename is historical, not a current Plonky2 prover.

## Source of truth

Examples in these pages are derived from the current CLI and configuration code. Confirm deployment-specific addresses, RPC endpoints, keys, authority-set values, and contract/program versions before using a command in production. The checked-in example files use placeholders or environment-specific values and are not production credentials.

The main implementation entry points are [`tools/beefy-relay`](../tools/beefy-relay), [`relayer/src/main.rs`](../relayer/src/main.rs), and [`relayer/src/cli/mod.rs`](../relayer/src/cli/mod.rs). When a guide and the CLI disagree, the compiled CLI and its validation logic take precedence.

## Scope and trust model

The outbound ZK verifier/prover pipeline has been removed from the current implementation; BEEFY/MMR authentication does not replace the independent inbound Beacon/receipt checks. Operating infrastructure still has security requirements. Follow the dedicated Gear-node guidance in the root README and keep fee-payer keys, web-server tokens, proof material, and persistent state private. Retained VFT/VFT-VARA and Ethereum token identities are unchanged; the production compatibility route is `EscrowV1` only, with no wrapper migration or held/v2 production path. If a manager is replaced, it must be newly constructed, never an in-place code/state upgrade. These documents and branch finalization authorize no production execution and do not replace deployment review, audits, or network-specific runbooks.
