# Gear Bridge documentation

This directory documents the bridge as it is implemented in this repository. The root [README](../README.md) remains the short project overview; these pages explain how to run, operate, and reason about the relayers and proving pipeline.

## Start here

- [Running the bridge](running-the-bridge.md): build prerequisites, configuration, Docker, startup, and shutdown.
- [Usage and operations](usage-and-operations.md): relayer modes, token workflows, manual operations, monitoring, and recovery.
- [Internals](internals.md): component boundaries and end-to-end message flow.
- [Merkle roots](merkle-roots.md): root extraction, accumulation, proving, persistence, and submission.
- [Circuits and proofs](circuits-and-proofs.md): the prover, Plonky2 circuits, trie proofs, and verification boundaries.

## Source of truth

Examples in these pages are derived from the current CLI and configuration code. Confirm deployment-specific addresses, RPC endpoints, keys, authority-set values, and contract/program versions before using a command in production. The checked-in example files use placeholders or environment-specific values and are not production credentials.

The main implementation entry points are [`relayer/src/main.rs`](../relayer/src/main.rs), [`relayer/src/cli/mod.rs`](../relayer/src/cli/mod.rs), and [`relayer/src/merkle_roots/`](../relayer/src/merkle_roots/). When a guide and the CLI disagree, the compiled CLI and its validation logic take precedence.

## Scope and trust model

The bridge is designed as a trustless ZK-based protocol, but operating infrastructure still has security requirements. In particular, follow the dedicated Gear-node guidance in the root README and keep fee-payer keys, web-server tokens, proof material, and persistent state private. These documents describe repository behavior; they do not replace deployment review, audits, or network-specific runbooks.
