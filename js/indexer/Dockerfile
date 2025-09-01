FROM node:20-alpine AS node
RUN apk update
RUN apk add g++ make python3

FROM node AS builder
COPY package.json yarn.lock lerna.json .yarnrc.yml /app/
COPY .yarn /app/.yarn
COPY api /app/api
COPY js/indexer /app/js/indexer
COPY js/common /app/js/common
WORKDIR /app
RUN yarn workspaces focus indexer gear-bridges gear-bridge-common
RUN yarn build:indexer

FROM node AS deps
WORKDIR /app
COPY package.json yarn.lock lerna.json .yarnrc.yml /app/
COPY .yarn /app/.yarn
COPY js/indexer /app/js/indexer
COPY js/common /app/js/common
RUN yarn workspaces focus gear-bridges indexer gear-bridge-common --production

FROM deps
COPY --from=builder /app/js/indexer/lib /app/js/indexer/lib
COPY --from=builder /app/js/indexer/db /app/js/indexer/db
COPY --from=builder /app/js/common/lib /app/js/common/lib
COPY --from=builder /app/api /app/api
WORKDIR /app
