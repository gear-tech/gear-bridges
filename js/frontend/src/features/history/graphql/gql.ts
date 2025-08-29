/* eslint-disable */
import * as types from './graphql';
import { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';

/**
 * Map of all GraphQL operations in the project.
 *
 * This map has several performance disadvantages:
 * 1. It is not tree-shakeable, so it will include all operations in the project.
 * 2. It is not minifiable, so the string of a GraphQL query will be multiple times inside the bundle.
 * 3. It does not support dead code elimination, so it will add unused operations.
 *
 * Therefore it is highly recommended to use the babel or swc plugin for production.
 * Learn more about it here: https://the-guild.dev/graphql/codegen/plugins/presets/preset-client#reducing-bundle-size
 */
type Documents = {
  '\n  query TransfersQuery($first: Int!, $offset: Int!, $filter: TransferFilter) {\n    allTransfers(first: $first, offset: $offset, orderBy: TIMESTAMP_DESC, filter: $filter) {\n      nodes {\n        amount\n        txHash\n        destNetwork\n        destination\n        id\n        receiver\n        sender\n        source\n        sourceNetwork\n        status\n        timestamp\n        nonce\n        blockNumber\n      }\n\n      totalCount\n    }\n  }\n': typeof types.TransfersQueryDocument;
  '\n  query PairsQuery {\n    allPairs {\n      nodes {\n        ethToken\n        ethTokenDecimals\n        ethTokenName\n        ethTokenSymbol\n        id\n        isActive\n        tokenSupply\n        varaToken\n        varaTokenDecimals\n        varaTokenName\n        varaTokenSymbol\n      }\n    }\n  }\n': typeof types.PairsQueryDocument;
  '\n  query TransferQuery($id: String!) {\n    transferById(id: $id) {\n      id\n      txHash\n      blockNumber\n      timestamp\n      completedAt\n      completedAtBlock\n      completedAtTxHash\n      nonce\n      sourceNetwork\n      source\n      destNetwork\n      destination\n      status\n      sender\n      receiver\n      amount\n      bridgingStartedAtBlock\n      bridgingStartedAtMessageId\n    }\n  }\n': typeof types.TransferQueryDocument;
  '\n  query TransfersCountQuery($filter: TransferFilter) {\n    allTransfers(filter: $filter) {\n      totalCount\n    }\n  }\n': typeof types.TransfersCountQueryDocument;
  '\n  query CheckpointSlotsQuery($slot: BigInt!) {\n    allCheckpointSlots(filter: { slot: { greaterThanOrEqualTo: $slot } }) {\n      totalCount\n    }\n  }\n': typeof types.CheckpointSlotsQueryDocument;
  '\n  query MerkelRootInMessageQueuesQuery($blockNumber: BigInt!) {\n    allMerkleRootInMessageQueues(filter: { blockNumber: { greaterThanOrEqualTo: $blockNumber } }) {\n      totalCount\n    }\n  }\n': typeof types.MerkelRootInMessageQueuesQueryDocument;
};
const documents: Documents = {
  '\n  query TransfersQuery($first: Int!, $offset: Int!, $filter: TransferFilter) {\n    allTransfers(first: $first, offset: $offset, orderBy: TIMESTAMP_DESC, filter: $filter) {\n      nodes {\n        amount\n        txHash\n        destNetwork\n        destination\n        id\n        receiver\n        sender\n        source\n        sourceNetwork\n        status\n        timestamp\n        nonce\n        blockNumber\n      }\n\n      totalCount\n    }\n  }\n':
    types.TransfersQueryDocument,
  '\n  query PairsQuery {\n    allPairs {\n      nodes {\n        ethToken\n        ethTokenDecimals\n        ethTokenName\n        ethTokenSymbol\n        id\n        isActive\n        tokenSupply\n        varaToken\n        varaTokenDecimals\n        varaTokenName\n        varaTokenSymbol\n      }\n    }\n  }\n':
    types.PairsQueryDocument,
  '\n  query TransferQuery($id: String!) {\n    transferById(id: $id) {\n      id\n      txHash\n      blockNumber\n      timestamp\n      completedAt\n      completedAtBlock\n      completedAtTxHash\n      nonce\n      sourceNetwork\n      source\n      destNetwork\n      destination\n      status\n      sender\n      receiver\n      amount\n      bridgingStartedAtBlock\n      bridgingStartedAtMessageId\n    }\n  }\n':
    types.TransferQueryDocument,
  '\n  query TransfersCountQuery($filter: TransferFilter) {\n    allTransfers(filter: $filter) {\n      totalCount\n    }\n  }\n':
    types.TransfersCountQueryDocument,
  '\n  query CheckpointSlotsQuery($slot: BigInt!) {\n    allCheckpointSlots(filter: { slot: { greaterThanOrEqualTo: $slot } }) {\n      totalCount\n    }\n  }\n':
    types.CheckpointSlotsQueryDocument,
  '\n  query MerkelRootInMessageQueuesQuery($blockNumber: BigInt!) {\n    allMerkleRootInMessageQueues(filter: { blockNumber: { greaterThanOrEqualTo: $blockNumber } }) {\n      totalCount\n    }\n  }\n':
    types.MerkelRootInMessageQueuesQueryDocument,
};

/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 *
 *
 * @example
 * ```ts
 * const query = graphql(`query GetUser($id: ID!) { user(id: $id) { name } }`);
 * ```
 *
 * The query argument is unknown!
 * Please regenerate the types.
 */
export function graphql(source: string): unknown;

/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(
  source: '\n  query TransfersQuery($first: Int!, $offset: Int!, $filter: TransferFilter) {\n    allTransfers(first: $first, offset: $offset, orderBy: TIMESTAMP_DESC, filter: $filter) {\n      nodes {\n        amount\n        txHash\n        destNetwork\n        destination\n        id\n        receiver\n        sender\n        source\n        sourceNetwork\n        status\n        timestamp\n        nonce\n        blockNumber\n      }\n\n      totalCount\n    }\n  }\n',
): (typeof documents)['\n  query TransfersQuery($first: Int!, $offset: Int!, $filter: TransferFilter) {\n    allTransfers(first: $first, offset: $offset, orderBy: TIMESTAMP_DESC, filter: $filter) {\n      nodes {\n        amount\n        txHash\n        destNetwork\n        destination\n        id\n        receiver\n        sender\n        source\n        sourceNetwork\n        status\n        timestamp\n        nonce\n        blockNumber\n      }\n\n      totalCount\n    }\n  }\n'];
/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(
  source: '\n  query PairsQuery {\n    allPairs {\n      nodes {\n        ethToken\n        ethTokenDecimals\n        ethTokenName\n        ethTokenSymbol\n        id\n        isActive\n        tokenSupply\n        varaToken\n        varaTokenDecimals\n        varaTokenName\n        varaTokenSymbol\n      }\n    }\n  }\n',
): (typeof documents)['\n  query PairsQuery {\n    allPairs {\n      nodes {\n        ethToken\n        ethTokenDecimals\n        ethTokenName\n        ethTokenSymbol\n        id\n        isActive\n        tokenSupply\n        varaToken\n        varaTokenDecimals\n        varaTokenName\n        varaTokenSymbol\n      }\n    }\n  }\n'];
/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(
  source: '\n  query TransferQuery($id: String!) {\n    transferById(id: $id) {\n      id\n      txHash\n      blockNumber\n      timestamp\n      completedAt\n      completedAtBlock\n      completedAtTxHash\n      nonce\n      sourceNetwork\n      source\n      destNetwork\n      destination\n      status\n      sender\n      receiver\n      amount\n      bridgingStartedAtBlock\n      bridgingStartedAtMessageId\n    }\n  }\n',
): (typeof documents)['\n  query TransferQuery($id: String!) {\n    transferById(id: $id) {\n      id\n      txHash\n      blockNumber\n      timestamp\n      completedAt\n      completedAtBlock\n      completedAtTxHash\n      nonce\n      sourceNetwork\n      source\n      destNetwork\n      destination\n      status\n      sender\n      receiver\n      amount\n      bridgingStartedAtBlock\n      bridgingStartedAtMessageId\n    }\n  }\n'];
/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(
  source: '\n  query TransfersCountQuery($filter: TransferFilter) {\n    allTransfers(filter: $filter) {\n      totalCount\n    }\n  }\n',
): (typeof documents)['\n  query TransfersCountQuery($filter: TransferFilter) {\n    allTransfers(filter: $filter) {\n      totalCount\n    }\n  }\n'];
/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(
  source: '\n  query CheckpointSlotsQuery($slot: BigInt!) {\n    allCheckpointSlots(filter: { slot: { greaterThanOrEqualTo: $slot } }) {\n      totalCount\n    }\n  }\n',
): (typeof documents)['\n  query CheckpointSlotsQuery($slot: BigInt!) {\n    allCheckpointSlots(filter: { slot: { greaterThanOrEqualTo: $slot } }) {\n      totalCount\n    }\n  }\n'];
/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(
  source: '\n  query MerkelRootInMessageQueuesQuery($blockNumber: BigInt!) {\n    allMerkleRootInMessageQueues(filter: { blockNumber: { greaterThanOrEqualTo: $blockNumber } }) {\n      totalCount\n    }\n  }\n',
): (typeof documents)['\n  query MerkelRootInMessageQueuesQuery($blockNumber: BigInt!) {\n    allMerkleRootInMessageQueues(filter: { blockNumber: { greaterThanOrEqualTo: $blockNumber } }) {\n      totalCount\n    }\n  }\n'];

export function graphql(source: string) {
  return (documents as any)[source] ?? {};
}

export type DocumentType<TDocumentNode extends DocumentNode<any, any>> =
  TDocumentNode extends DocumentNode<infer TType, any> ? TType : never;
