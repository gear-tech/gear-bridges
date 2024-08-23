/* eslint-disable */
import { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';
export type Maybe<T> = T | null;
export type InputMaybe<T> = Maybe<T>;
export type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };
export type MakeOptional<T, K extends keyof T> = Omit<T, K> & { [SubKey in K]?: Maybe<T[SubKey]> };
export type MakeMaybe<T, K extends keyof T> = Omit<T, K> & { [SubKey in K]: Maybe<T[SubKey]> };
export type MakeEmpty<T extends { [key: string]: unknown }, K extends keyof T> = { [_ in K]?: never };
export type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };
/** All built-in and custom scalars, mapped to their actual values */
export type Scalars = {
  ID: { input: string; output: string; }
  String: { input: string; output: string; }
  Boolean: { input: boolean; output: boolean; }
  Int: { input: number; output: number; }
  Float: { input: number; output: number; }
  /** Big number integer */
  BigInt: { input: string; output: string; }
  /** A date-time string in simplified extended ISO 8601 format (YYYY-MM-DDTHH:mm:ss.sssZ) */
  DateTime: { input: string; output: string; }
};

export enum Direction {
  EthToVara = 'EthToVara',
  VaraToEth = 'VaraToEth'
}

export type NotUpdatedCompleted = {
  __typename?: 'NotUpdatedCompleted';
  id: Scalars['String']['output'];
  side: Side;
};

export type NotUpdatedCompletedEdge = {
  __typename?: 'NotUpdatedCompletedEdge';
  cursor: Scalars['String']['output'];
  node: NotUpdatedCompleted;
};

export enum NotUpdatedCompletedOrderByInput {
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdDesc = 'id_DESC',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  SideAsc = 'side_ASC',
  SideAscNullsFirst = 'side_ASC_NULLS_FIRST',
  SideDesc = 'side_DESC',
  SideDescNullsLast = 'side_DESC_NULLS_LAST'
}

export type NotUpdatedCompletedWhereInput = {
  AND: InputMaybe<Array<NotUpdatedCompletedWhereInput>>;
  OR: InputMaybe<Array<NotUpdatedCompletedWhereInput>>;
  id_contains: InputMaybe<Scalars['String']['input']>;
  id_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  id_endsWith: InputMaybe<Scalars['String']['input']>;
  id_eq: InputMaybe<Scalars['String']['input']>;
  id_gt: InputMaybe<Scalars['String']['input']>;
  id_gte: InputMaybe<Scalars['String']['input']>;
  id_in: InputMaybe<Array<Scalars['String']['input']>>;
  id_isNull: InputMaybe<Scalars['Boolean']['input']>;
  id_lt: InputMaybe<Scalars['String']['input']>;
  id_lte: InputMaybe<Scalars['String']['input']>;
  id_not_contains: InputMaybe<Scalars['String']['input']>;
  id_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  id_not_endsWith: InputMaybe<Scalars['String']['input']>;
  id_not_eq: InputMaybe<Scalars['String']['input']>;
  id_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  id_not_startsWith: InputMaybe<Scalars['String']['input']>;
  id_startsWith: InputMaybe<Scalars['String']['input']>;
  side_eq: InputMaybe<Side>;
  side_in: InputMaybe<Array<Side>>;
  side_isNull: InputMaybe<Scalars['Boolean']['input']>;
  side_not_eq: InputMaybe<Side>;
  side_not_in: InputMaybe<Array<Side>>;
};

export type NotUpdatedCompletedsConnection = {
  __typename?: 'NotUpdatedCompletedsConnection';
  edges: Array<NotUpdatedCompletedEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export type PageInfo = {
  __typename?: 'PageInfo';
  endCursor: Scalars['String']['output'];
  hasNextPage: Scalars['Boolean']['output'];
  hasPreviousPage: Scalars['Boolean']['output'];
  startCursor: Scalars['String']['output'];
};

export enum Pair {
  EthWrappedEth = 'EthWrappedEth',
  UsdcWrappedUsdc = 'USDCWrappedUSDC',
  UsdtWrappedUsdt = 'USDTWrappedUSDT',
  VaraWrappedVara = 'VaraWrappedVara'
}

export type Query = {
  __typename?: 'Query';
  notUpdatedCompletedById: Maybe<NotUpdatedCompleted>;
  /** @deprecated Use notUpdatedCompletedById */
  notUpdatedCompletedByUniqueInput: Maybe<NotUpdatedCompleted>;
  notUpdatedCompleteds: Array<NotUpdatedCompleted>;
  notUpdatedCompletedsConnection: NotUpdatedCompletedsConnection;
  squidStatus: Maybe<SquidStatus>;
  teleportById: Maybe<Teleport>;
  /** @deprecated Use teleportById */
  teleportByUniqueInput: Maybe<Teleport>;
  teleports: Array<Teleport>;
  teleportsConnection: TeleportsConnection;
};


export type QueryNotUpdatedCompletedByIdArgs = {
  id: Scalars['String']['input'];
};


export type QueryNotUpdatedCompletedByUniqueInputArgs = {
  where: WhereIdInput;
};


export type QueryNotUpdatedCompletedsArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<NotUpdatedCompletedOrderByInput>>;
  where: InputMaybe<NotUpdatedCompletedWhereInput>;
};


export type QueryNotUpdatedCompletedsConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<NotUpdatedCompletedOrderByInput>;
  where: InputMaybe<NotUpdatedCompletedWhereInput>;
};


export type QueryTeleportByIdArgs = {
  id: Scalars['String']['input'];
};


export type QueryTeleportByUniqueInputArgs = {
  where: WhereIdInput;
};


export type QueryTeleportsArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<TeleportOrderByInput>>;
  where: InputMaybe<TeleportWhereInput>;
};


export type QueryTeleportsConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<TeleportOrderByInput>;
  where: InputMaybe<TeleportWhereInput>;
};

export enum Side {
  Eth = 'Eth',
  Vara = 'Vara'
}

export type SquidStatus = {
  __typename?: 'SquidStatus';
  /** The height of the processed part of the chain */
  height: Maybe<Scalars['Int']['output']>;
};

export enum Status {
  Completed = 'Completed',
  InProgress = 'InProgress'
}

export type Teleport = {
  __typename?: 'Teleport';
  amount: Scalars['BigInt']['output'];
  block: Scalars['BigInt']['output'];
  blockhash: Scalars['String']['output'];
  direction: Direction;
  from: Scalars['String']['output'];
  id: Scalars['String']['output'];
  nonce: Scalars['BigInt']['output'];
  pair: Pair;
  status: Status;
  timestamp: Scalars['DateTime']['output'];
  to: Scalars['String']['output'];
};

export type TeleportEdge = {
  __typename?: 'TeleportEdge';
  cursor: Scalars['String']['output'];
  node: Teleport;
};

export enum TeleportOrderByInput {
  AmountAsc = 'amount_ASC',
  AmountAscNullsFirst = 'amount_ASC_NULLS_FIRST',
  AmountDesc = 'amount_DESC',
  AmountDescNullsLast = 'amount_DESC_NULLS_LAST',
  BlockAsc = 'block_ASC',
  BlockAscNullsFirst = 'block_ASC_NULLS_FIRST',
  BlockDesc = 'block_DESC',
  BlockDescNullsLast = 'block_DESC_NULLS_LAST',
  BlockhashAsc = 'blockhash_ASC',
  BlockhashAscNullsFirst = 'blockhash_ASC_NULLS_FIRST',
  BlockhashDesc = 'blockhash_DESC',
  BlockhashDescNullsLast = 'blockhash_DESC_NULLS_LAST',
  DirectionAsc = 'direction_ASC',
  DirectionAscNullsFirst = 'direction_ASC_NULLS_FIRST',
  DirectionDesc = 'direction_DESC',
  DirectionDescNullsLast = 'direction_DESC_NULLS_LAST',
  FromAsc = 'from_ASC',
  FromAscNullsFirst = 'from_ASC_NULLS_FIRST',
  FromDesc = 'from_DESC',
  FromDescNullsLast = 'from_DESC_NULLS_LAST',
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdDesc = 'id_DESC',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  NonceAsc = 'nonce_ASC',
  NonceAscNullsFirst = 'nonce_ASC_NULLS_FIRST',
  NonceDesc = 'nonce_DESC',
  NonceDescNullsLast = 'nonce_DESC_NULLS_LAST',
  PairAsc = 'pair_ASC',
  PairAscNullsFirst = 'pair_ASC_NULLS_FIRST',
  PairDesc = 'pair_DESC',
  PairDescNullsLast = 'pair_DESC_NULLS_LAST',
  StatusAsc = 'status_ASC',
  StatusAscNullsFirst = 'status_ASC_NULLS_FIRST',
  StatusDesc = 'status_DESC',
  StatusDescNullsLast = 'status_DESC_NULLS_LAST',
  TimestampAsc = 'timestamp_ASC',
  TimestampAscNullsFirst = 'timestamp_ASC_NULLS_FIRST',
  TimestampDesc = 'timestamp_DESC',
  TimestampDescNullsLast = 'timestamp_DESC_NULLS_LAST',
  ToAsc = 'to_ASC',
  ToAscNullsFirst = 'to_ASC_NULLS_FIRST',
  ToDesc = 'to_DESC',
  ToDescNullsLast = 'to_DESC_NULLS_LAST'
}

export type TeleportWhereInput = {
  AND: InputMaybe<Array<TeleportWhereInput>>;
  OR: InputMaybe<Array<TeleportWhereInput>>;
  amount_eq: InputMaybe<Scalars['BigInt']['input']>;
  amount_gt: InputMaybe<Scalars['BigInt']['input']>;
  amount_gte: InputMaybe<Scalars['BigInt']['input']>;
  amount_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  amount_isNull: InputMaybe<Scalars['Boolean']['input']>;
  amount_lt: InputMaybe<Scalars['BigInt']['input']>;
  amount_lte: InputMaybe<Scalars['BigInt']['input']>;
  amount_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  amount_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  block_eq: InputMaybe<Scalars['BigInt']['input']>;
  block_gt: InputMaybe<Scalars['BigInt']['input']>;
  block_gte: InputMaybe<Scalars['BigInt']['input']>;
  block_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  block_isNull: InputMaybe<Scalars['Boolean']['input']>;
  block_lt: InputMaybe<Scalars['BigInt']['input']>;
  block_lte: InputMaybe<Scalars['BigInt']['input']>;
  block_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  block_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  blockhash_contains: InputMaybe<Scalars['String']['input']>;
  blockhash_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  blockhash_endsWith: InputMaybe<Scalars['String']['input']>;
  blockhash_eq: InputMaybe<Scalars['String']['input']>;
  blockhash_gt: InputMaybe<Scalars['String']['input']>;
  blockhash_gte: InputMaybe<Scalars['String']['input']>;
  blockhash_in: InputMaybe<Array<Scalars['String']['input']>>;
  blockhash_isNull: InputMaybe<Scalars['Boolean']['input']>;
  blockhash_lt: InputMaybe<Scalars['String']['input']>;
  blockhash_lte: InputMaybe<Scalars['String']['input']>;
  blockhash_not_contains: InputMaybe<Scalars['String']['input']>;
  blockhash_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  blockhash_not_endsWith: InputMaybe<Scalars['String']['input']>;
  blockhash_not_eq: InputMaybe<Scalars['String']['input']>;
  blockhash_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  blockhash_not_startsWith: InputMaybe<Scalars['String']['input']>;
  blockhash_startsWith: InputMaybe<Scalars['String']['input']>;
  direction_eq: InputMaybe<Direction>;
  direction_in: InputMaybe<Array<Direction>>;
  direction_isNull: InputMaybe<Scalars['Boolean']['input']>;
  direction_not_eq: InputMaybe<Direction>;
  direction_not_in: InputMaybe<Array<Direction>>;
  from_contains: InputMaybe<Scalars['String']['input']>;
  from_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  from_endsWith: InputMaybe<Scalars['String']['input']>;
  from_eq: InputMaybe<Scalars['String']['input']>;
  from_gt: InputMaybe<Scalars['String']['input']>;
  from_gte: InputMaybe<Scalars['String']['input']>;
  from_in: InputMaybe<Array<Scalars['String']['input']>>;
  from_isNull: InputMaybe<Scalars['Boolean']['input']>;
  from_lt: InputMaybe<Scalars['String']['input']>;
  from_lte: InputMaybe<Scalars['String']['input']>;
  from_not_contains: InputMaybe<Scalars['String']['input']>;
  from_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  from_not_endsWith: InputMaybe<Scalars['String']['input']>;
  from_not_eq: InputMaybe<Scalars['String']['input']>;
  from_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  from_not_startsWith: InputMaybe<Scalars['String']['input']>;
  from_startsWith: InputMaybe<Scalars['String']['input']>;
  id_contains: InputMaybe<Scalars['String']['input']>;
  id_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  id_endsWith: InputMaybe<Scalars['String']['input']>;
  id_eq: InputMaybe<Scalars['String']['input']>;
  id_gt: InputMaybe<Scalars['String']['input']>;
  id_gte: InputMaybe<Scalars['String']['input']>;
  id_in: InputMaybe<Array<Scalars['String']['input']>>;
  id_isNull: InputMaybe<Scalars['Boolean']['input']>;
  id_lt: InputMaybe<Scalars['String']['input']>;
  id_lte: InputMaybe<Scalars['String']['input']>;
  id_not_contains: InputMaybe<Scalars['String']['input']>;
  id_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  id_not_endsWith: InputMaybe<Scalars['String']['input']>;
  id_not_eq: InputMaybe<Scalars['String']['input']>;
  id_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  id_not_startsWith: InputMaybe<Scalars['String']['input']>;
  id_startsWith: InputMaybe<Scalars['String']['input']>;
  nonce_eq: InputMaybe<Scalars['BigInt']['input']>;
  nonce_gt: InputMaybe<Scalars['BigInt']['input']>;
  nonce_gte: InputMaybe<Scalars['BigInt']['input']>;
  nonce_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  nonce_isNull: InputMaybe<Scalars['Boolean']['input']>;
  nonce_lt: InputMaybe<Scalars['BigInt']['input']>;
  nonce_lte: InputMaybe<Scalars['BigInt']['input']>;
  nonce_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  nonce_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  pair_eq: InputMaybe<Pair>;
  pair_in: InputMaybe<Array<Pair>>;
  pair_isNull: InputMaybe<Scalars['Boolean']['input']>;
  pair_not_eq: InputMaybe<Pair>;
  pair_not_in: InputMaybe<Array<Pair>>;
  status_eq: InputMaybe<Status>;
  status_in: InputMaybe<Array<Status>>;
  status_isNull: InputMaybe<Scalars['Boolean']['input']>;
  status_not_eq: InputMaybe<Status>;
  status_not_in: InputMaybe<Array<Status>>;
  timestamp_eq: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_gt: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_gte: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_in: InputMaybe<Array<Scalars['DateTime']['input']>>;
  timestamp_isNull: InputMaybe<Scalars['Boolean']['input']>;
  timestamp_lt: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_lte: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_not_eq: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_not_in: InputMaybe<Array<Scalars['DateTime']['input']>>;
  to_contains: InputMaybe<Scalars['String']['input']>;
  to_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  to_endsWith: InputMaybe<Scalars['String']['input']>;
  to_eq: InputMaybe<Scalars['String']['input']>;
  to_gt: InputMaybe<Scalars['String']['input']>;
  to_gte: InputMaybe<Scalars['String']['input']>;
  to_in: InputMaybe<Array<Scalars['String']['input']>>;
  to_isNull: InputMaybe<Scalars['Boolean']['input']>;
  to_lt: InputMaybe<Scalars['String']['input']>;
  to_lte: InputMaybe<Scalars['String']['input']>;
  to_not_contains: InputMaybe<Scalars['String']['input']>;
  to_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  to_not_endsWith: InputMaybe<Scalars['String']['input']>;
  to_not_eq: InputMaybe<Scalars['String']['input']>;
  to_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  to_not_startsWith: InputMaybe<Scalars['String']['input']>;
  to_startsWith: InputMaybe<Scalars['String']['input']>;
};

export type TeleportsConnection = {
  __typename?: 'TeleportsConnection';
  edges: Array<TeleportEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export type WhereIdInput = {
  id: Scalars['String']['input'];
};

export type TeleportsQueryQueryVariables = Exact<{
  limit: Scalars['Int']['input'];
  offset: Scalars['Int']['input'];
  where: InputMaybe<TeleportWhereInput>;
}>;


export type TeleportsQueryQuery = { __typename?: 'Query', teleports: Array<{ __typename?: 'Teleport', amount: string, blockhash: string, direction: Direction, from: string, id: string, status: Status, timestamp: string, to: string, pair: Pair }> };

export type TeleportsConnectionQueryQueryVariables = Exact<{
  where: InputMaybe<TeleportWhereInput>;
}>;


export type TeleportsConnectionQueryQuery = { __typename?: 'Query', teleportsConnection: { __typename?: 'TeleportsConnection', totalCount: number } };


export const TeleportsQueryDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"TeleportsQuery"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"limit"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"offset"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"where"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"TeleportWhereInput"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"teleports"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"limit"},"value":{"kind":"Variable","name":{"kind":"Name","value":"limit"}}},{"kind":"Argument","name":{"kind":"Name","value":"offset"},"value":{"kind":"Variable","name":{"kind":"Name","value":"offset"}}},{"kind":"Argument","name":{"kind":"Name","value":"orderBy"},"value":{"kind":"EnumValue","value":"timestamp_DESC"}},{"kind":"Argument","name":{"kind":"Name","value":"where"},"value":{"kind":"Variable","name":{"kind":"Name","value":"where"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"amount"}},{"kind":"Field","name":{"kind":"Name","value":"blockhash"}},{"kind":"Field","name":{"kind":"Name","value":"direction"}},{"kind":"Field","name":{"kind":"Name","value":"from"}},{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"timestamp"}},{"kind":"Field","name":{"kind":"Name","value":"to"}},{"kind":"Field","name":{"kind":"Name","value":"pair"}}]}}]}}]} as unknown as DocumentNode<TeleportsQueryQuery, TeleportsQueryQueryVariables>;
export const TeleportsConnectionQueryDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"TeleportsConnectionQuery"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"where"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"TeleportWhereInput"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"teleportsConnection"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"orderBy"},"value":{"kind":"EnumValue","value":"timestamp_DESC"}},{"kind":"Argument","name":{"kind":"Name","value":"where"},"value":{"kind":"Variable","name":{"kind":"Name","value":"where"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalCount"}}]}}]}}]} as unknown as DocumentNode<TeleportsConnectionQueryQuery, TeleportsConnectionQueryQueryVariables>;