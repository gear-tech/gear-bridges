# Vara Network Bridge

## Deployment configuration

Frontend settings are compiled by Vite; changing container runtime environment variables does not update the served bundle. Supply the network and contract settings from `.env.example` at build time. Docker accepts the same settings as build arguments, including:

- `VITE_INBOUND_PROOF_PROFILES`: independently approved `MAINNET`/`TESTNET` inbound profile JSON matching the deployed contracts, programs and checkpoint trust pins. Never put secrets in frontend configuration.
- `VITE_OWNED_FIXTURE_NETWORK`: the approved local-consensus descriptor for an isolated chain-31337 fixture only. Leave unset for Hoodi and Ethereum mainnet.

Missing deployment configuration holds the entire application; missing or mismatched inbound approval disables Ethereum-to-Vara submission/claims rather than asserting settlement. A successful image build is not transfer qualification: configure the matching live stack and verify recipient balances and finalized settlement in both directions.

The existing header selector chooses Mainnet or Testnet; URL/storage selection is retained and the default remains Mainnet. Explicitly select Testnet for Hoodi/local-Vara qualification and supply only its independently approved profile; never reuse Hoodi pins as mainnet approval.

For a loopback preview with a separate GraphQL API, use the preview origin for the testnet indexer URL and Vite's native `preview.proxy` for `/graphql` to the loopback API. This avoids broadening backend CORS or exposing the API. Verify the actual configured browser, live token pairs and indexed transfer states; a paid transfer shown as Bridging is not proof of destination delivery.

Vara queue block/message identifiers apply only to Vara-origin transfers. Ethereum-origin transfers show their native destination settlement under Timings rather than permanent loading placeholders for those source-only fields.

Transaction details re-fetch every 60 seconds until the indexer provides a settlement timestamp. This also fills settlement metadata after an optimistic status update; status alone does not stop refresh.

The Docker frontend build uses a 4 GiB Node heap; the default 2 GiB heap is insufficient for the current bundle.
