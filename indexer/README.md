# Multichain transfers squid

This [squid](https://docs.subsquid.io/) captures USDC Transfer events on ETH and BSC, stores them in the same database and serves the data over a common GraphQL API.

The Ethereum processor is located in `src/eth` and similarly the Binance Chain processor can be found in `src/bsc`. The scripts file `commands.json` was updated with the commands `process:eth` and `process:bsc` to run the processors. 

You can find some useful hints on developing multichain squids on the [dedicated documentation page](https://docs.subsquid.io/basics/multichain/).

Dependencies: Node.js, Docker, Git.

## Quickstart

```bash
# 0. Install @subsquid/cli a.k.a. the sqd command globally
npm i -g @subsquid/cli

# 1. Clone the repo
git clone https://github.com/subsquid-labs/multichain-transfers-example
cd multichain-transfers-example

# 2. Install dependencies
npm ci

# 3. Start a Postgres database container and detach
sqd up

# 4. Apply the migration
sqd migration:apply

# 5. Build the squid
sqd build

# 6. Run all services at once
sqd run .
```
A GraphiQL playground will be available at [localhost:4350/graphql](http://localhost:4350/graphql).

You can also run individual services separately:
```bash
sqd process:eth # Ethereum processor
sqd process:bsc # BSC processor
sqd serve       # GraphQL server
```
