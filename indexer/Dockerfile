FROM node:20-alpine AS node

FROM node AS api
WORKDIR /assets
COPY api/gear/vft_manager.idl .
COPY api/gear/historical_proxy.idl .
COPY api/gear/bridging_payment.idl .
COPY api/gear/vft.idl .
COPY api/ethereum/IERC20Manager.json .
COPY api/ethereum/IMessageQueue.json .
COPY api/ethereum/IBridgingPayment.json .

FROM node AS node-with-gyp
RUN apk add g++ make python3
FROM node-with-gyp AS builder
WORKDIR /squid
COPY indexer/package.json .
COPY indexer/package-lock.json .
COPY indexer/db db
COPY indexer/schema.graphql .
RUN npm i -g @subsquid/cli && npm ci
COPY indexer/tsconfig.json .
COPY indexer/src src
COPY --from=api /assets assets
RUN npm run build
RUN ls -ltr

FROM node-with-gyp AS deps
WORKDIR /squid
COPY indexer/package.json .
COPY indexer/package-lock.json .
RUN npm ci --production

FROM node AS squid
WORKDIR /squid
COPY --from=deps /squid/package.json .
COPY --from=deps /squid/package-lock.json .
COPY --from=deps /squid/node_modules node_modules
COPY --from=builder /squid/lib lib
COPY --from=builder /squid/db db
COPY --from=builder /squid/schema.graphql schema.graphql
COPY --from=api /assets assets
COPY indexer/commands.json .
RUN echo -e "loglevel=silent\\nupdate-notifier=false" >/squid/.npmrc
RUN npm i -g @subsquid/commands && mv $(which squid-commands) /usr/local/bin/sqd
ENV PROCESSOR_PROMETHEUS_PORT=3000
