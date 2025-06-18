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
  ID: { input: string; output: string };
  String: { input: string; output: string };
  Boolean: { input: boolean; output: boolean };
  Int: { input: number; output: number };
  Float: { input: number; output: number };
  /** Big number integer */
  BigInt: { input: string; output: string };
  /** A date-time string in simplified extended ISO 8601 format (YYYY-MM-DDTHH:mm:ss.sssZ) */
  DateTime: { input: string; output: string };
};

export type CompletedTransfer = {
  __typename?: 'CompletedTransfer';
  destNetwork: Network;
  id: Scalars['String']['output'];
  nonce: Scalars['String']['output'];
  timestamp: Maybe<Scalars['DateTime']['output']>;
};

export type CompletedTransferEdge = {
  __typename?: 'CompletedTransferEdge';
  cursor: Scalars['String']['output'];
  node: CompletedTransfer;
};

export enum CompletedTransferOrderByInput {
  DestNetworkAsc = 'destNetwork_ASC',
  DestNetworkAscNullsFirst = 'destNetwork_ASC_NULLS_FIRST',
  DestNetworkAscNullsLast = 'destNetwork_ASC_NULLS_LAST',
  DestNetworkDesc = 'destNetwork_DESC',
  DestNetworkDescNullsFirst = 'destNetwork_DESC_NULLS_FIRST',
  DestNetworkDescNullsLast = 'destNetwork_DESC_NULLS_LAST',
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdAscNullsLast = 'id_ASC_NULLS_LAST',
  IdDesc = 'id_DESC',
  IdDescNullsFirst = 'id_DESC_NULLS_FIRST',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  NonceAsc = 'nonce_ASC',
  NonceAscNullsFirst = 'nonce_ASC_NULLS_FIRST',
  NonceAscNullsLast = 'nonce_ASC_NULLS_LAST',
  NonceDesc = 'nonce_DESC',
  NonceDescNullsFirst = 'nonce_DESC_NULLS_FIRST',
  NonceDescNullsLast = 'nonce_DESC_NULLS_LAST',
  TimestampAsc = 'timestamp_ASC',
  TimestampAscNullsFirst = 'timestamp_ASC_NULLS_FIRST',
  TimestampAscNullsLast = 'timestamp_ASC_NULLS_LAST',
  TimestampDesc = 'timestamp_DESC',
  TimestampDescNullsFirst = 'timestamp_DESC_NULLS_FIRST',
  TimestampDescNullsLast = 'timestamp_DESC_NULLS_LAST',
}

export type CompletedTransferWhereInput = {
  AND: InputMaybe<Array<CompletedTransferWhereInput>>;
  OR: InputMaybe<Array<CompletedTransferWhereInput>>;
  destNetwork_eq: InputMaybe<Network>;
  destNetwork_in: InputMaybe<Array<Network>>;
  destNetwork_isNull: InputMaybe<Scalars['Boolean']['input']>;
  destNetwork_not_eq: InputMaybe<Network>;
  destNetwork_not_in: InputMaybe<Array<Network>>;
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
  nonce_contains: InputMaybe<Scalars['String']['input']>;
  nonce_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  nonce_endsWith: InputMaybe<Scalars['String']['input']>;
  nonce_eq: InputMaybe<Scalars['String']['input']>;
  nonce_gt: InputMaybe<Scalars['String']['input']>;
  nonce_gte: InputMaybe<Scalars['String']['input']>;
  nonce_in: InputMaybe<Array<Scalars['String']['input']>>;
  nonce_isNull: InputMaybe<Scalars['Boolean']['input']>;
  nonce_lt: InputMaybe<Scalars['String']['input']>;
  nonce_lte: InputMaybe<Scalars['String']['input']>;
  nonce_not_contains: InputMaybe<Scalars['String']['input']>;
  nonce_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  nonce_not_endsWith: InputMaybe<Scalars['String']['input']>;
  nonce_not_eq: InputMaybe<Scalars['String']['input']>;
  nonce_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  nonce_not_startsWith: InputMaybe<Scalars['String']['input']>;
  nonce_startsWith: InputMaybe<Scalars['String']['input']>;
  timestamp_eq: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_gt: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_gte: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_in: InputMaybe<Array<Scalars['DateTime']['input']>>;
  timestamp_isNull: InputMaybe<Scalars['Boolean']['input']>;
  timestamp_lt: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_lte: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_not_eq: InputMaybe<Scalars['DateTime']['input']>;
  timestamp_not_in: InputMaybe<Array<Scalars['DateTime']['input']>>;
};

export type CompletedTransfersConnection = {
  __typename?: 'CompletedTransfersConnection';
  edges: Array<CompletedTransferEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export type EthBridgeProgram = {
  __typename?: 'EthBridgeProgram';
  id: Scalars['String']['output'];
  name: Scalars['String']['output'];
};

export type EthBridgeProgramEdge = {
  __typename?: 'EthBridgeProgramEdge';
  cursor: Scalars['String']['output'];
  node: EthBridgeProgram;
};

export enum EthBridgeProgramOrderByInput {
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdAscNullsLast = 'id_ASC_NULLS_LAST',
  IdDesc = 'id_DESC',
  IdDescNullsFirst = 'id_DESC_NULLS_FIRST',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  NameAsc = 'name_ASC',
  NameAscNullsFirst = 'name_ASC_NULLS_FIRST',
  NameAscNullsLast = 'name_ASC_NULLS_LAST',
  NameDesc = 'name_DESC',
  NameDescNullsFirst = 'name_DESC_NULLS_FIRST',
  NameDescNullsLast = 'name_DESC_NULLS_LAST',
}

export type EthBridgeProgramWhereInput = {
  AND: InputMaybe<Array<EthBridgeProgramWhereInput>>;
  OR: InputMaybe<Array<EthBridgeProgramWhereInput>>;
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
  name_contains: InputMaybe<Scalars['String']['input']>;
  name_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  name_endsWith: InputMaybe<Scalars['String']['input']>;
  name_eq: InputMaybe<Scalars['String']['input']>;
  name_gt: InputMaybe<Scalars['String']['input']>;
  name_gte: InputMaybe<Scalars['String']['input']>;
  name_in: InputMaybe<Array<Scalars['String']['input']>>;
  name_isNull: InputMaybe<Scalars['Boolean']['input']>;
  name_lt: InputMaybe<Scalars['String']['input']>;
  name_lte: InputMaybe<Scalars['String']['input']>;
  name_not_contains: InputMaybe<Scalars['String']['input']>;
  name_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  name_not_endsWith: InputMaybe<Scalars['String']['input']>;
  name_not_eq: InputMaybe<Scalars['String']['input']>;
  name_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  name_not_startsWith: InputMaybe<Scalars['String']['input']>;
  name_startsWith: InputMaybe<Scalars['String']['input']>;
};

export type EthBridgeProgramsConnection = {
  __typename?: 'EthBridgeProgramsConnection';
  edges: Array<EthBridgeProgramEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export enum Network {
  Ethereum = 'Ethereum',
  Vara = 'Vara',
}

export type PageInfo = {
  __typename?: 'PageInfo';
  endCursor: Scalars['String']['output'];
  hasNextPage: Scalars['Boolean']['output'];
  hasPreviousPage: Scalars['Boolean']['output'];
  startCursor: Scalars['String']['output'];
};

export type Pair = {
  __typename?: 'Pair';
  activeSinceBlock: Scalars['BigInt']['output'];
  activeToBlock: Maybe<Scalars['BigInt']['output']>;
  ethToken: Scalars['String']['output'];
  ethTokenDecimals: Scalars['Int']['output'];
  ethTokenName: Scalars['String']['output'];
  ethTokenSymbol: Scalars['String']['output'];
  id: Scalars['String']['output'];
  isActive: Scalars['Boolean']['output'];
  isRemoved: Scalars['Boolean']['output'];
  tokenSupply: Network;
  upgradedTo: Maybe<Scalars['String']['output']>;
  varaToken: Scalars['String']['output'];
  varaTokenDecimals: Scalars['Int']['output'];
  varaTokenName: Scalars['String']['output'];
  varaTokenSymbol: Scalars['String']['output'];
};

export type PairEdge = {
  __typename?: 'PairEdge';
  cursor: Scalars['String']['output'];
  node: Pair;
};

export enum PairOrderByInput {
  ActiveSinceBlockAsc = 'activeSinceBlock_ASC',
  ActiveSinceBlockAscNullsFirst = 'activeSinceBlock_ASC_NULLS_FIRST',
  ActiveSinceBlockAscNullsLast = 'activeSinceBlock_ASC_NULLS_LAST',
  ActiveSinceBlockDesc = 'activeSinceBlock_DESC',
  ActiveSinceBlockDescNullsFirst = 'activeSinceBlock_DESC_NULLS_FIRST',
  ActiveSinceBlockDescNullsLast = 'activeSinceBlock_DESC_NULLS_LAST',
  ActiveToBlockAsc = 'activeToBlock_ASC',
  ActiveToBlockAscNullsFirst = 'activeToBlock_ASC_NULLS_FIRST',
  ActiveToBlockAscNullsLast = 'activeToBlock_ASC_NULLS_LAST',
  ActiveToBlockDesc = 'activeToBlock_DESC',
  ActiveToBlockDescNullsFirst = 'activeToBlock_DESC_NULLS_FIRST',
  ActiveToBlockDescNullsLast = 'activeToBlock_DESC_NULLS_LAST',
  EthTokenDecimalsAsc = 'ethTokenDecimals_ASC',
  EthTokenDecimalsAscNullsFirst = 'ethTokenDecimals_ASC_NULLS_FIRST',
  EthTokenDecimalsAscNullsLast = 'ethTokenDecimals_ASC_NULLS_LAST',
  EthTokenDecimalsDesc = 'ethTokenDecimals_DESC',
  EthTokenDecimalsDescNullsFirst = 'ethTokenDecimals_DESC_NULLS_FIRST',
  EthTokenDecimalsDescNullsLast = 'ethTokenDecimals_DESC_NULLS_LAST',
  EthTokenNameAsc = 'ethTokenName_ASC',
  EthTokenNameAscNullsFirst = 'ethTokenName_ASC_NULLS_FIRST',
  EthTokenNameAscNullsLast = 'ethTokenName_ASC_NULLS_LAST',
  EthTokenNameDesc = 'ethTokenName_DESC',
  EthTokenNameDescNullsFirst = 'ethTokenName_DESC_NULLS_FIRST',
  EthTokenNameDescNullsLast = 'ethTokenName_DESC_NULLS_LAST',
  EthTokenSymbolAsc = 'ethTokenSymbol_ASC',
  EthTokenSymbolAscNullsFirst = 'ethTokenSymbol_ASC_NULLS_FIRST',
  EthTokenSymbolAscNullsLast = 'ethTokenSymbol_ASC_NULLS_LAST',
  EthTokenSymbolDesc = 'ethTokenSymbol_DESC',
  EthTokenSymbolDescNullsFirst = 'ethTokenSymbol_DESC_NULLS_FIRST',
  EthTokenSymbolDescNullsLast = 'ethTokenSymbol_DESC_NULLS_LAST',
  EthTokenAsc = 'ethToken_ASC',
  EthTokenAscNullsFirst = 'ethToken_ASC_NULLS_FIRST',
  EthTokenAscNullsLast = 'ethToken_ASC_NULLS_LAST',
  EthTokenDesc = 'ethToken_DESC',
  EthTokenDescNullsFirst = 'ethToken_DESC_NULLS_FIRST',
  EthTokenDescNullsLast = 'ethToken_DESC_NULLS_LAST',
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdAscNullsLast = 'id_ASC_NULLS_LAST',
  IdDesc = 'id_DESC',
  IdDescNullsFirst = 'id_DESC_NULLS_FIRST',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  IsActiveAsc = 'isActive_ASC',
  IsActiveAscNullsFirst = 'isActive_ASC_NULLS_FIRST',
  IsActiveAscNullsLast = 'isActive_ASC_NULLS_LAST',
  IsActiveDesc = 'isActive_DESC',
  IsActiveDescNullsFirst = 'isActive_DESC_NULLS_FIRST',
  IsActiveDescNullsLast = 'isActive_DESC_NULLS_LAST',
  IsRemovedAsc = 'isRemoved_ASC',
  IsRemovedAscNullsFirst = 'isRemoved_ASC_NULLS_FIRST',
  IsRemovedAscNullsLast = 'isRemoved_ASC_NULLS_LAST',
  IsRemovedDesc = 'isRemoved_DESC',
  IsRemovedDescNullsFirst = 'isRemoved_DESC_NULLS_FIRST',
  IsRemovedDescNullsLast = 'isRemoved_DESC_NULLS_LAST',
  TokenSupplyAsc = 'tokenSupply_ASC',
  TokenSupplyAscNullsFirst = 'tokenSupply_ASC_NULLS_FIRST',
  TokenSupplyAscNullsLast = 'tokenSupply_ASC_NULLS_LAST',
  TokenSupplyDesc = 'tokenSupply_DESC',
  TokenSupplyDescNullsFirst = 'tokenSupply_DESC_NULLS_FIRST',
  TokenSupplyDescNullsLast = 'tokenSupply_DESC_NULLS_LAST',
  UpgradedToAsc = 'upgradedTo_ASC',
  UpgradedToAscNullsFirst = 'upgradedTo_ASC_NULLS_FIRST',
  UpgradedToAscNullsLast = 'upgradedTo_ASC_NULLS_LAST',
  UpgradedToDesc = 'upgradedTo_DESC',
  UpgradedToDescNullsFirst = 'upgradedTo_DESC_NULLS_FIRST',
  UpgradedToDescNullsLast = 'upgradedTo_DESC_NULLS_LAST',
  VaraTokenDecimalsAsc = 'varaTokenDecimals_ASC',
  VaraTokenDecimalsAscNullsFirst = 'varaTokenDecimals_ASC_NULLS_FIRST',
  VaraTokenDecimalsAscNullsLast = 'varaTokenDecimals_ASC_NULLS_LAST',
  VaraTokenDecimalsDesc = 'varaTokenDecimals_DESC',
  VaraTokenDecimalsDescNullsFirst = 'varaTokenDecimals_DESC_NULLS_FIRST',
  VaraTokenDecimalsDescNullsLast = 'varaTokenDecimals_DESC_NULLS_LAST',
  VaraTokenNameAsc = 'varaTokenName_ASC',
  VaraTokenNameAscNullsFirst = 'varaTokenName_ASC_NULLS_FIRST',
  VaraTokenNameAscNullsLast = 'varaTokenName_ASC_NULLS_LAST',
  VaraTokenNameDesc = 'varaTokenName_DESC',
  VaraTokenNameDescNullsFirst = 'varaTokenName_DESC_NULLS_FIRST',
  VaraTokenNameDescNullsLast = 'varaTokenName_DESC_NULLS_LAST',
  VaraTokenSymbolAsc = 'varaTokenSymbol_ASC',
  VaraTokenSymbolAscNullsFirst = 'varaTokenSymbol_ASC_NULLS_FIRST',
  VaraTokenSymbolAscNullsLast = 'varaTokenSymbol_ASC_NULLS_LAST',
  VaraTokenSymbolDesc = 'varaTokenSymbol_DESC',
  VaraTokenSymbolDescNullsFirst = 'varaTokenSymbol_DESC_NULLS_FIRST',
  VaraTokenSymbolDescNullsLast = 'varaTokenSymbol_DESC_NULLS_LAST',
  VaraTokenAsc = 'varaToken_ASC',
  VaraTokenAscNullsFirst = 'varaToken_ASC_NULLS_FIRST',
  VaraTokenAscNullsLast = 'varaToken_ASC_NULLS_LAST',
  VaraTokenDesc = 'varaToken_DESC',
  VaraTokenDescNullsFirst = 'varaToken_DESC_NULLS_FIRST',
  VaraTokenDescNullsLast = 'varaToken_DESC_NULLS_LAST',
}

export type PairWhereInput = {
  AND: InputMaybe<Array<PairWhereInput>>;
  OR: InputMaybe<Array<PairWhereInput>>;
  activeSinceBlock_eq: InputMaybe<Scalars['BigInt']['input']>;
  activeSinceBlock_gt: InputMaybe<Scalars['BigInt']['input']>;
  activeSinceBlock_gte: InputMaybe<Scalars['BigInt']['input']>;
  activeSinceBlock_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  activeSinceBlock_isNull: InputMaybe<Scalars['Boolean']['input']>;
  activeSinceBlock_lt: InputMaybe<Scalars['BigInt']['input']>;
  activeSinceBlock_lte: InputMaybe<Scalars['BigInt']['input']>;
  activeSinceBlock_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  activeSinceBlock_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  activeToBlock_eq: InputMaybe<Scalars['BigInt']['input']>;
  activeToBlock_gt: InputMaybe<Scalars['BigInt']['input']>;
  activeToBlock_gte: InputMaybe<Scalars['BigInt']['input']>;
  activeToBlock_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  activeToBlock_isNull: InputMaybe<Scalars['Boolean']['input']>;
  activeToBlock_lt: InputMaybe<Scalars['BigInt']['input']>;
  activeToBlock_lte: InputMaybe<Scalars['BigInt']['input']>;
  activeToBlock_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  activeToBlock_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  ethTokenDecimals_eq: InputMaybe<Scalars['Int']['input']>;
  ethTokenDecimals_gt: InputMaybe<Scalars['Int']['input']>;
  ethTokenDecimals_gte: InputMaybe<Scalars['Int']['input']>;
  ethTokenDecimals_in: InputMaybe<Array<Scalars['Int']['input']>>;
  ethTokenDecimals_isNull: InputMaybe<Scalars['Boolean']['input']>;
  ethTokenDecimals_lt: InputMaybe<Scalars['Int']['input']>;
  ethTokenDecimals_lte: InputMaybe<Scalars['Int']['input']>;
  ethTokenDecimals_not_eq: InputMaybe<Scalars['Int']['input']>;
  ethTokenDecimals_not_in: InputMaybe<Array<Scalars['Int']['input']>>;
  ethTokenName_contains: InputMaybe<Scalars['String']['input']>;
  ethTokenName_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  ethTokenName_endsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenName_eq: InputMaybe<Scalars['String']['input']>;
  ethTokenName_gt: InputMaybe<Scalars['String']['input']>;
  ethTokenName_gte: InputMaybe<Scalars['String']['input']>;
  ethTokenName_in: InputMaybe<Array<Scalars['String']['input']>>;
  ethTokenName_isNull: InputMaybe<Scalars['Boolean']['input']>;
  ethTokenName_lt: InputMaybe<Scalars['String']['input']>;
  ethTokenName_lte: InputMaybe<Scalars['String']['input']>;
  ethTokenName_not_contains: InputMaybe<Scalars['String']['input']>;
  ethTokenName_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  ethTokenName_not_endsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenName_not_eq: InputMaybe<Scalars['String']['input']>;
  ethTokenName_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  ethTokenName_not_startsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenName_startsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_contains: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_endsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_eq: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_gt: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_gte: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_in: InputMaybe<Array<Scalars['String']['input']>>;
  ethTokenSymbol_isNull: InputMaybe<Scalars['Boolean']['input']>;
  ethTokenSymbol_lt: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_lte: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_not_contains: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_not_endsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_not_eq: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  ethTokenSymbol_not_startsWith: InputMaybe<Scalars['String']['input']>;
  ethTokenSymbol_startsWith: InputMaybe<Scalars['String']['input']>;
  ethToken_contains: InputMaybe<Scalars['String']['input']>;
  ethToken_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  ethToken_endsWith: InputMaybe<Scalars['String']['input']>;
  ethToken_eq: InputMaybe<Scalars['String']['input']>;
  ethToken_gt: InputMaybe<Scalars['String']['input']>;
  ethToken_gte: InputMaybe<Scalars['String']['input']>;
  ethToken_in: InputMaybe<Array<Scalars['String']['input']>>;
  ethToken_isNull: InputMaybe<Scalars['Boolean']['input']>;
  ethToken_lt: InputMaybe<Scalars['String']['input']>;
  ethToken_lte: InputMaybe<Scalars['String']['input']>;
  ethToken_not_contains: InputMaybe<Scalars['String']['input']>;
  ethToken_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  ethToken_not_endsWith: InputMaybe<Scalars['String']['input']>;
  ethToken_not_eq: InputMaybe<Scalars['String']['input']>;
  ethToken_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  ethToken_not_startsWith: InputMaybe<Scalars['String']['input']>;
  ethToken_startsWith: InputMaybe<Scalars['String']['input']>;
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
  isActive_eq: InputMaybe<Scalars['Boolean']['input']>;
  isActive_isNull: InputMaybe<Scalars['Boolean']['input']>;
  isActive_not_eq: InputMaybe<Scalars['Boolean']['input']>;
  isRemoved_eq: InputMaybe<Scalars['Boolean']['input']>;
  isRemoved_isNull: InputMaybe<Scalars['Boolean']['input']>;
  isRemoved_not_eq: InputMaybe<Scalars['Boolean']['input']>;
  tokenSupply_eq: InputMaybe<Network>;
  tokenSupply_in: InputMaybe<Array<Network>>;
  tokenSupply_isNull: InputMaybe<Scalars['Boolean']['input']>;
  tokenSupply_not_eq: InputMaybe<Network>;
  tokenSupply_not_in: InputMaybe<Array<Network>>;
  upgradedTo_contains: InputMaybe<Scalars['String']['input']>;
  upgradedTo_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  upgradedTo_endsWith: InputMaybe<Scalars['String']['input']>;
  upgradedTo_eq: InputMaybe<Scalars['String']['input']>;
  upgradedTo_gt: InputMaybe<Scalars['String']['input']>;
  upgradedTo_gte: InputMaybe<Scalars['String']['input']>;
  upgradedTo_in: InputMaybe<Array<Scalars['String']['input']>>;
  upgradedTo_isNull: InputMaybe<Scalars['Boolean']['input']>;
  upgradedTo_lt: InputMaybe<Scalars['String']['input']>;
  upgradedTo_lte: InputMaybe<Scalars['String']['input']>;
  upgradedTo_not_contains: InputMaybe<Scalars['String']['input']>;
  upgradedTo_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  upgradedTo_not_endsWith: InputMaybe<Scalars['String']['input']>;
  upgradedTo_not_eq: InputMaybe<Scalars['String']['input']>;
  upgradedTo_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  upgradedTo_not_startsWith: InputMaybe<Scalars['String']['input']>;
  upgradedTo_startsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenDecimals_eq: InputMaybe<Scalars['Int']['input']>;
  varaTokenDecimals_gt: InputMaybe<Scalars['Int']['input']>;
  varaTokenDecimals_gte: InputMaybe<Scalars['Int']['input']>;
  varaTokenDecimals_in: InputMaybe<Array<Scalars['Int']['input']>>;
  varaTokenDecimals_isNull: InputMaybe<Scalars['Boolean']['input']>;
  varaTokenDecimals_lt: InputMaybe<Scalars['Int']['input']>;
  varaTokenDecimals_lte: InputMaybe<Scalars['Int']['input']>;
  varaTokenDecimals_not_eq: InputMaybe<Scalars['Int']['input']>;
  varaTokenDecimals_not_in: InputMaybe<Array<Scalars['Int']['input']>>;
  varaTokenName_contains: InputMaybe<Scalars['String']['input']>;
  varaTokenName_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  varaTokenName_endsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenName_eq: InputMaybe<Scalars['String']['input']>;
  varaTokenName_gt: InputMaybe<Scalars['String']['input']>;
  varaTokenName_gte: InputMaybe<Scalars['String']['input']>;
  varaTokenName_in: InputMaybe<Array<Scalars['String']['input']>>;
  varaTokenName_isNull: InputMaybe<Scalars['Boolean']['input']>;
  varaTokenName_lt: InputMaybe<Scalars['String']['input']>;
  varaTokenName_lte: InputMaybe<Scalars['String']['input']>;
  varaTokenName_not_contains: InputMaybe<Scalars['String']['input']>;
  varaTokenName_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  varaTokenName_not_endsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenName_not_eq: InputMaybe<Scalars['String']['input']>;
  varaTokenName_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  varaTokenName_not_startsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenName_startsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_contains: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_endsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_eq: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_gt: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_gte: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_in: InputMaybe<Array<Scalars['String']['input']>>;
  varaTokenSymbol_isNull: InputMaybe<Scalars['Boolean']['input']>;
  varaTokenSymbol_lt: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_lte: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_not_contains: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_not_endsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_not_eq: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  varaTokenSymbol_not_startsWith: InputMaybe<Scalars['String']['input']>;
  varaTokenSymbol_startsWith: InputMaybe<Scalars['String']['input']>;
  varaToken_contains: InputMaybe<Scalars['String']['input']>;
  varaToken_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  varaToken_endsWith: InputMaybe<Scalars['String']['input']>;
  varaToken_eq: InputMaybe<Scalars['String']['input']>;
  varaToken_gt: InputMaybe<Scalars['String']['input']>;
  varaToken_gte: InputMaybe<Scalars['String']['input']>;
  varaToken_in: InputMaybe<Array<Scalars['String']['input']>>;
  varaToken_isNull: InputMaybe<Scalars['Boolean']['input']>;
  varaToken_lt: InputMaybe<Scalars['String']['input']>;
  varaToken_lte: InputMaybe<Scalars['String']['input']>;
  varaToken_not_contains: InputMaybe<Scalars['String']['input']>;
  varaToken_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  varaToken_not_endsWith: InputMaybe<Scalars['String']['input']>;
  varaToken_not_eq: InputMaybe<Scalars['String']['input']>;
  varaToken_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  varaToken_not_startsWith: InputMaybe<Scalars['String']['input']>;
  varaToken_startsWith: InputMaybe<Scalars['String']['input']>;
};

export type PairsConnection = {
  __typename?: 'PairsConnection';
  edges: Array<PairEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export type Query = {
  __typename?: 'Query';
  completedTransferById: Maybe<CompletedTransfer>;
  completedTransfers: Array<CompletedTransfer>;
  completedTransfersConnection: CompletedTransfersConnection;
  ethBridgeProgramById: Maybe<EthBridgeProgram>;
  ethBridgePrograms: Array<EthBridgeProgram>;
  ethBridgeProgramsConnection: EthBridgeProgramsConnection;
  pairById: Maybe<Pair>;
  pairs: Array<Pair>;
  pairsConnection: PairsConnection;
  squidStatus: Maybe<SquidStatus>;
  transferById: Maybe<Transfer>;
  transfers: Array<Transfer>;
  transfersConnection: TransfersConnection;
  varaBridgeProgramById: Maybe<VaraBridgeProgram>;
  varaBridgePrograms: Array<VaraBridgeProgram>;
  varaBridgeProgramsConnection: VaraBridgeProgramsConnection;
};

export type QueryCompletedTransferByIdArgs = {
  id: Scalars['String']['input'];
};

export type QueryCompletedTransfersArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<CompletedTransferOrderByInput>>;
  where: InputMaybe<CompletedTransferWhereInput>;
};

export type QueryCompletedTransfersConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<CompletedTransferOrderByInput>;
  where: InputMaybe<CompletedTransferWhereInput>;
};

export type QueryEthBridgeProgramByIdArgs = {
  id: Scalars['String']['input'];
};

export type QueryEthBridgeProgramsArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<EthBridgeProgramOrderByInput>>;
  where: InputMaybe<EthBridgeProgramWhereInput>;
};

export type QueryEthBridgeProgramsConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<EthBridgeProgramOrderByInput>;
  where: InputMaybe<EthBridgeProgramWhereInput>;
};

export type QueryPairByIdArgs = {
  id: Scalars['String']['input'];
};

export type QueryPairsArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<PairOrderByInput>>;
  where: InputMaybe<PairWhereInput>;
};

export type QueryPairsConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<PairOrderByInput>;
  where: InputMaybe<PairWhereInput>;
};

export type QueryTransferByIdArgs = {
  id: Scalars['String']['input'];
};

export type QueryTransfersArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<TransferOrderByInput>>;
  where: InputMaybe<TransferWhereInput>;
};

export type QueryTransfersConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<TransferOrderByInput>;
  where: InputMaybe<TransferWhereInput>;
};

export type QueryVaraBridgeProgramByIdArgs = {
  id: Scalars['String']['input'];
};

export type QueryVaraBridgeProgramsArgs = {
  limit: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<VaraBridgeProgramOrderByInput>>;
  where: InputMaybe<VaraBridgeProgramWhereInput>;
};

export type QueryVaraBridgeProgramsConnectionArgs = {
  after: InputMaybe<Scalars['String']['input']>;
  first: InputMaybe<Scalars['Int']['input']>;
  orderBy: Array<VaraBridgeProgramOrderByInput>;
  where: InputMaybe<VaraBridgeProgramWhereInput>;
};

export type SquidStatus = {
  __typename?: 'SquidStatus';
  /** The hash of the last processed finalized block */
  finalizedHash: Maybe<Scalars['String']['output']>;
  /** The height of the last processed finalized block */
  finalizedHeight: Maybe<Scalars['Int']['output']>;
  /** The hash of the last processed block */
  hash: Maybe<Scalars['String']['output']>;
  /** The height of the last processed block */
  height: Maybe<Scalars['Int']['output']>;
};

export enum Status {
  AwaitingPayment = 'AwaitingPayment',
  Bridging = 'Bridging',
  Completed = 'Completed',
  Failed = 'Failed',
}

export type Transfer = {
  __typename?: 'Transfer';
  amount: Scalars['BigInt']['output'];
  blockNumber: Scalars['BigInt']['output'];
  completedAt: Maybe<Scalars['DateTime']['output']>;
  destNetwork: Network;
  destination: Scalars['String']['output'];
  id: Scalars['String']['output'];
  nonce: Scalars['String']['output'];
  receiver: Scalars['String']['output'];
  sender: Scalars['String']['output'];
  source: Scalars['String']['output'];
  sourceNetwork: Network;
  status: Status;
  timestamp: Scalars['DateTime']['output'];
  txHash: Scalars['String']['output'];
};

export type TransferEdge = {
  __typename?: 'TransferEdge';
  cursor: Scalars['String']['output'];
  node: Transfer;
};

export enum TransferOrderByInput {
  AmountAsc = 'amount_ASC',
  AmountAscNullsFirst = 'amount_ASC_NULLS_FIRST',
  AmountAscNullsLast = 'amount_ASC_NULLS_LAST',
  AmountDesc = 'amount_DESC',
  AmountDescNullsFirst = 'amount_DESC_NULLS_FIRST',
  AmountDescNullsLast = 'amount_DESC_NULLS_LAST',
  BlockNumberAsc = 'blockNumber_ASC',
  BlockNumberAscNullsFirst = 'blockNumber_ASC_NULLS_FIRST',
  BlockNumberAscNullsLast = 'blockNumber_ASC_NULLS_LAST',
  BlockNumberDesc = 'blockNumber_DESC',
  BlockNumberDescNullsFirst = 'blockNumber_DESC_NULLS_FIRST',
  BlockNumberDescNullsLast = 'blockNumber_DESC_NULLS_LAST',
  CompletedAtAsc = 'completedAt_ASC',
  CompletedAtAscNullsFirst = 'completedAt_ASC_NULLS_FIRST',
  CompletedAtAscNullsLast = 'completedAt_ASC_NULLS_LAST',
  CompletedAtDesc = 'completedAt_DESC',
  CompletedAtDescNullsFirst = 'completedAt_DESC_NULLS_FIRST',
  CompletedAtDescNullsLast = 'completedAt_DESC_NULLS_LAST',
  DestNetworkAsc = 'destNetwork_ASC',
  DestNetworkAscNullsFirst = 'destNetwork_ASC_NULLS_FIRST',
  DestNetworkAscNullsLast = 'destNetwork_ASC_NULLS_LAST',
  DestNetworkDesc = 'destNetwork_DESC',
  DestNetworkDescNullsFirst = 'destNetwork_DESC_NULLS_FIRST',
  DestNetworkDescNullsLast = 'destNetwork_DESC_NULLS_LAST',
  DestinationAsc = 'destination_ASC',
  DestinationAscNullsFirst = 'destination_ASC_NULLS_FIRST',
  DestinationAscNullsLast = 'destination_ASC_NULLS_LAST',
  DestinationDesc = 'destination_DESC',
  DestinationDescNullsFirst = 'destination_DESC_NULLS_FIRST',
  DestinationDescNullsLast = 'destination_DESC_NULLS_LAST',
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdAscNullsLast = 'id_ASC_NULLS_LAST',
  IdDesc = 'id_DESC',
  IdDescNullsFirst = 'id_DESC_NULLS_FIRST',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  NonceAsc = 'nonce_ASC',
  NonceAscNullsFirst = 'nonce_ASC_NULLS_FIRST',
  NonceAscNullsLast = 'nonce_ASC_NULLS_LAST',
  NonceDesc = 'nonce_DESC',
  NonceDescNullsFirst = 'nonce_DESC_NULLS_FIRST',
  NonceDescNullsLast = 'nonce_DESC_NULLS_LAST',
  ReceiverAsc = 'receiver_ASC',
  ReceiverAscNullsFirst = 'receiver_ASC_NULLS_FIRST',
  ReceiverAscNullsLast = 'receiver_ASC_NULLS_LAST',
  ReceiverDesc = 'receiver_DESC',
  ReceiverDescNullsFirst = 'receiver_DESC_NULLS_FIRST',
  ReceiverDescNullsLast = 'receiver_DESC_NULLS_LAST',
  SenderAsc = 'sender_ASC',
  SenderAscNullsFirst = 'sender_ASC_NULLS_FIRST',
  SenderAscNullsLast = 'sender_ASC_NULLS_LAST',
  SenderDesc = 'sender_DESC',
  SenderDescNullsFirst = 'sender_DESC_NULLS_FIRST',
  SenderDescNullsLast = 'sender_DESC_NULLS_LAST',
  SourceNetworkAsc = 'sourceNetwork_ASC',
  SourceNetworkAscNullsFirst = 'sourceNetwork_ASC_NULLS_FIRST',
  SourceNetworkAscNullsLast = 'sourceNetwork_ASC_NULLS_LAST',
  SourceNetworkDesc = 'sourceNetwork_DESC',
  SourceNetworkDescNullsFirst = 'sourceNetwork_DESC_NULLS_FIRST',
  SourceNetworkDescNullsLast = 'sourceNetwork_DESC_NULLS_LAST',
  SourceAsc = 'source_ASC',
  SourceAscNullsFirst = 'source_ASC_NULLS_FIRST',
  SourceAscNullsLast = 'source_ASC_NULLS_LAST',
  SourceDesc = 'source_DESC',
  SourceDescNullsFirst = 'source_DESC_NULLS_FIRST',
  SourceDescNullsLast = 'source_DESC_NULLS_LAST',
  StatusAsc = 'status_ASC',
  StatusAscNullsFirst = 'status_ASC_NULLS_FIRST',
  StatusAscNullsLast = 'status_ASC_NULLS_LAST',
  StatusDesc = 'status_DESC',
  StatusDescNullsFirst = 'status_DESC_NULLS_FIRST',
  StatusDescNullsLast = 'status_DESC_NULLS_LAST',
  TimestampAsc = 'timestamp_ASC',
  TimestampAscNullsFirst = 'timestamp_ASC_NULLS_FIRST',
  TimestampAscNullsLast = 'timestamp_ASC_NULLS_LAST',
  TimestampDesc = 'timestamp_DESC',
  TimestampDescNullsFirst = 'timestamp_DESC_NULLS_FIRST',
  TimestampDescNullsLast = 'timestamp_DESC_NULLS_LAST',
  TxHashAsc = 'txHash_ASC',
  TxHashAscNullsFirst = 'txHash_ASC_NULLS_FIRST',
  TxHashAscNullsLast = 'txHash_ASC_NULLS_LAST',
  TxHashDesc = 'txHash_DESC',
  TxHashDescNullsFirst = 'txHash_DESC_NULLS_FIRST',
  TxHashDescNullsLast = 'txHash_DESC_NULLS_LAST',
}

export type TransferWhereInput = {
  AND: InputMaybe<Array<TransferWhereInput>>;
  OR: InputMaybe<Array<TransferWhereInput>>;
  amount_eq: InputMaybe<Scalars['BigInt']['input']>;
  amount_gt: InputMaybe<Scalars['BigInt']['input']>;
  amount_gte: InputMaybe<Scalars['BigInt']['input']>;
  amount_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  amount_isNull: InputMaybe<Scalars['Boolean']['input']>;
  amount_lt: InputMaybe<Scalars['BigInt']['input']>;
  amount_lte: InputMaybe<Scalars['BigInt']['input']>;
  amount_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  amount_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  blockNumber_eq: InputMaybe<Scalars['BigInt']['input']>;
  blockNumber_gt: InputMaybe<Scalars['BigInt']['input']>;
  blockNumber_gte: InputMaybe<Scalars['BigInt']['input']>;
  blockNumber_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  blockNumber_isNull: InputMaybe<Scalars['Boolean']['input']>;
  blockNumber_lt: InputMaybe<Scalars['BigInt']['input']>;
  blockNumber_lte: InputMaybe<Scalars['BigInt']['input']>;
  blockNumber_not_eq: InputMaybe<Scalars['BigInt']['input']>;
  blockNumber_not_in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  completedAt_eq: InputMaybe<Scalars['DateTime']['input']>;
  completedAt_gt: InputMaybe<Scalars['DateTime']['input']>;
  completedAt_gte: InputMaybe<Scalars['DateTime']['input']>;
  completedAt_in: InputMaybe<Array<Scalars['DateTime']['input']>>;
  completedAt_isNull: InputMaybe<Scalars['Boolean']['input']>;
  completedAt_lt: InputMaybe<Scalars['DateTime']['input']>;
  completedAt_lte: InputMaybe<Scalars['DateTime']['input']>;
  completedAt_not_eq: InputMaybe<Scalars['DateTime']['input']>;
  completedAt_not_in: InputMaybe<Array<Scalars['DateTime']['input']>>;
  destNetwork_eq: InputMaybe<Network>;
  destNetwork_in: InputMaybe<Array<Network>>;
  destNetwork_isNull: InputMaybe<Scalars['Boolean']['input']>;
  destNetwork_not_eq: InputMaybe<Network>;
  destNetwork_not_in: InputMaybe<Array<Network>>;
  destination_contains: InputMaybe<Scalars['String']['input']>;
  destination_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  destination_endsWith: InputMaybe<Scalars['String']['input']>;
  destination_eq: InputMaybe<Scalars['String']['input']>;
  destination_gt: InputMaybe<Scalars['String']['input']>;
  destination_gte: InputMaybe<Scalars['String']['input']>;
  destination_in: InputMaybe<Array<Scalars['String']['input']>>;
  destination_isNull: InputMaybe<Scalars['Boolean']['input']>;
  destination_lt: InputMaybe<Scalars['String']['input']>;
  destination_lte: InputMaybe<Scalars['String']['input']>;
  destination_not_contains: InputMaybe<Scalars['String']['input']>;
  destination_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  destination_not_endsWith: InputMaybe<Scalars['String']['input']>;
  destination_not_eq: InputMaybe<Scalars['String']['input']>;
  destination_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  destination_not_startsWith: InputMaybe<Scalars['String']['input']>;
  destination_startsWith: InputMaybe<Scalars['String']['input']>;
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
  nonce_contains: InputMaybe<Scalars['String']['input']>;
  nonce_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  nonce_endsWith: InputMaybe<Scalars['String']['input']>;
  nonce_eq: InputMaybe<Scalars['String']['input']>;
  nonce_gt: InputMaybe<Scalars['String']['input']>;
  nonce_gte: InputMaybe<Scalars['String']['input']>;
  nonce_in: InputMaybe<Array<Scalars['String']['input']>>;
  nonce_isNull: InputMaybe<Scalars['Boolean']['input']>;
  nonce_lt: InputMaybe<Scalars['String']['input']>;
  nonce_lte: InputMaybe<Scalars['String']['input']>;
  nonce_not_contains: InputMaybe<Scalars['String']['input']>;
  nonce_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  nonce_not_endsWith: InputMaybe<Scalars['String']['input']>;
  nonce_not_eq: InputMaybe<Scalars['String']['input']>;
  nonce_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  nonce_not_startsWith: InputMaybe<Scalars['String']['input']>;
  nonce_startsWith: InputMaybe<Scalars['String']['input']>;
  receiver_contains: InputMaybe<Scalars['String']['input']>;
  receiver_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  receiver_endsWith: InputMaybe<Scalars['String']['input']>;
  receiver_eq: InputMaybe<Scalars['String']['input']>;
  receiver_gt: InputMaybe<Scalars['String']['input']>;
  receiver_gte: InputMaybe<Scalars['String']['input']>;
  receiver_in: InputMaybe<Array<Scalars['String']['input']>>;
  receiver_isNull: InputMaybe<Scalars['Boolean']['input']>;
  receiver_lt: InputMaybe<Scalars['String']['input']>;
  receiver_lte: InputMaybe<Scalars['String']['input']>;
  receiver_not_contains: InputMaybe<Scalars['String']['input']>;
  receiver_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  receiver_not_endsWith: InputMaybe<Scalars['String']['input']>;
  receiver_not_eq: InputMaybe<Scalars['String']['input']>;
  receiver_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  receiver_not_startsWith: InputMaybe<Scalars['String']['input']>;
  receiver_startsWith: InputMaybe<Scalars['String']['input']>;
  sender_contains: InputMaybe<Scalars['String']['input']>;
  sender_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  sender_endsWith: InputMaybe<Scalars['String']['input']>;
  sender_eq: InputMaybe<Scalars['String']['input']>;
  sender_gt: InputMaybe<Scalars['String']['input']>;
  sender_gte: InputMaybe<Scalars['String']['input']>;
  sender_in: InputMaybe<Array<Scalars['String']['input']>>;
  sender_isNull: InputMaybe<Scalars['Boolean']['input']>;
  sender_lt: InputMaybe<Scalars['String']['input']>;
  sender_lte: InputMaybe<Scalars['String']['input']>;
  sender_not_contains: InputMaybe<Scalars['String']['input']>;
  sender_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  sender_not_endsWith: InputMaybe<Scalars['String']['input']>;
  sender_not_eq: InputMaybe<Scalars['String']['input']>;
  sender_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  sender_not_startsWith: InputMaybe<Scalars['String']['input']>;
  sender_startsWith: InputMaybe<Scalars['String']['input']>;
  sourceNetwork_eq: InputMaybe<Network>;
  sourceNetwork_in: InputMaybe<Array<Network>>;
  sourceNetwork_isNull: InputMaybe<Scalars['Boolean']['input']>;
  sourceNetwork_not_eq: InputMaybe<Network>;
  sourceNetwork_not_in: InputMaybe<Array<Network>>;
  source_contains: InputMaybe<Scalars['String']['input']>;
  source_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  source_endsWith: InputMaybe<Scalars['String']['input']>;
  source_eq: InputMaybe<Scalars['String']['input']>;
  source_gt: InputMaybe<Scalars['String']['input']>;
  source_gte: InputMaybe<Scalars['String']['input']>;
  source_in: InputMaybe<Array<Scalars['String']['input']>>;
  source_isNull: InputMaybe<Scalars['Boolean']['input']>;
  source_lt: InputMaybe<Scalars['String']['input']>;
  source_lte: InputMaybe<Scalars['String']['input']>;
  source_not_contains: InputMaybe<Scalars['String']['input']>;
  source_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  source_not_endsWith: InputMaybe<Scalars['String']['input']>;
  source_not_eq: InputMaybe<Scalars['String']['input']>;
  source_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  source_not_startsWith: InputMaybe<Scalars['String']['input']>;
  source_startsWith: InputMaybe<Scalars['String']['input']>;
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
  txHash_contains: InputMaybe<Scalars['String']['input']>;
  txHash_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  txHash_endsWith: InputMaybe<Scalars['String']['input']>;
  txHash_eq: InputMaybe<Scalars['String']['input']>;
  txHash_gt: InputMaybe<Scalars['String']['input']>;
  txHash_gte: InputMaybe<Scalars['String']['input']>;
  txHash_in: InputMaybe<Array<Scalars['String']['input']>>;
  txHash_isNull: InputMaybe<Scalars['Boolean']['input']>;
  txHash_lt: InputMaybe<Scalars['String']['input']>;
  txHash_lte: InputMaybe<Scalars['String']['input']>;
  txHash_not_contains: InputMaybe<Scalars['String']['input']>;
  txHash_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  txHash_not_endsWith: InputMaybe<Scalars['String']['input']>;
  txHash_not_eq: InputMaybe<Scalars['String']['input']>;
  txHash_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  txHash_not_startsWith: InputMaybe<Scalars['String']['input']>;
  txHash_startsWith: InputMaybe<Scalars['String']['input']>;
};

export type TransfersConnection = {
  __typename?: 'TransfersConnection';
  edges: Array<TransferEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export type VaraBridgeProgram = {
  __typename?: 'VaraBridgeProgram';
  id: Scalars['String']['output'];
  name: Scalars['String']['output'];
};

export type VaraBridgeProgramEdge = {
  __typename?: 'VaraBridgeProgramEdge';
  cursor: Scalars['String']['output'];
  node: VaraBridgeProgram;
};

export enum VaraBridgeProgramOrderByInput {
  IdAsc = 'id_ASC',
  IdAscNullsFirst = 'id_ASC_NULLS_FIRST',
  IdAscNullsLast = 'id_ASC_NULLS_LAST',
  IdDesc = 'id_DESC',
  IdDescNullsFirst = 'id_DESC_NULLS_FIRST',
  IdDescNullsLast = 'id_DESC_NULLS_LAST',
  NameAsc = 'name_ASC',
  NameAscNullsFirst = 'name_ASC_NULLS_FIRST',
  NameAscNullsLast = 'name_ASC_NULLS_LAST',
  NameDesc = 'name_DESC',
  NameDescNullsFirst = 'name_DESC_NULLS_FIRST',
  NameDescNullsLast = 'name_DESC_NULLS_LAST',
}

export type VaraBridgeProgramWhereInput = {
  AND: InputMaybe<Array<VaraBridgeProgramWhereInput>>;
  OR: InputMaybe<Array<VaraBridgeProgramWhereInput>>;
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
  name_contains: InputMaybe<Scalars['String']['input']>;
  name_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  name_endsWith: InputMaybe<Scalars['String']['input']>;
  name_eq: InputMaybe<Scalars['String']['input']>;
  name_gt: InputMaybe<Scalars['String']['input']>;
  name_gte: InputMaybe<Scalars['String']['input']>;
  name_in: InputMaybe<Array<Scalars['String']['input']>>;
  name_isNull: InputMaybe<Scalars['Boolean']['input']>;
  name_lt: InputMaybe<Scalars['String']['input']>;
  name_lte: InputMaybe<Scalars['String']['input']>;
  name_not_contains: InputMaybe<Scalars['String']['input']>;
  name_not_containsInsensitive: InputMaybe<Scalars['String']['input']>;
  name_not_endsWith: InputMaybe<Scalars['String']['input']>;
  name_not_eq: InputMaybe<Scalars['String']['input']>;
  name_not_in: InputMaybe<Array<Scalars['String']['input']>>;
  name_not_startsWith: InputMaybe<Scalars['String']['input']>;
  name_startsWith: InputMaybe<Scalars['String']['input']>;
};

export type VaraBridgeProgramsConnection = {
  __typename?: 'VaraBridgeProgramsConnection';
  edges: Array<VaraBridgeProgramEdge>;
  pageInfo: PageInfo;
  totalCount: Scalars['Int']['output'];
};

export type TransfersQueryQueryVariables = Exact<{
  limit: Scalars['Int']['input'];
  offset: Scalars['Int']['input'];
  where: InputMaybe<TransferWhereInput>;
}>;

export type TransfersQueryQuery = {
  __typename?: 'Query';
  transfers: Array<{
    __typename?: 'Transfer';
    amount: string;
    txHash: string;
    destNetwork: Network;
    destination: string;
    id: string;
    receiver: string;
    sender: string;
    source: string;
    sourceNetwork: Network;
    status: Status;
    timestamp: string;
    nonce: string;
    blockNumber: string;
  }>;
};

export type TransfersConnectionQueryQueryVariables = Exact<{
  where: InputMaybe<TransferWhereInput>;
}>;

export type TransfersConnectionQueryQuery = {
  __typename?: 'Query';
  transfersConnection: { __typename?: 'TransfersConnection'; totalCount: number };
};

export type PairsQueryQueryVariables = Exact<{ [key: string]: never }>;

export type PairsQueryQuery = {
  __typename?: 'Query';
  pairs: Array<{
    __typename?: 'Pair';
    ethToken: string;
    ethTokenDecimals: number;
    ethTokenName: string;
    ethTokenSymbol: string;
    id: string;
    isRemoved: boolean;
    tokenSupply: Network;
    varaToken: string;
    varaTokenDecimals: number;
    varaTokenName: string;
    varaTokenSymbol: string;
  }>;
};

export const TransfersQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'TransfersQuery' },
      variableDefinitions: [
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } } },
        },
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'offset' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } } },
        },
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'where' } },
          type: { kind: 'NamedType', name: { kind: 'Name', value: 'TransferWhereInput' } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'transfers' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'limit' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'offset' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'offset' } },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'orderBy' },
                value: { kind: 'EnumValue', value: 'timestamp_DESC' },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'where' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'where' } },
              },
            ],
            selectionSet: {
              kind: 'SelectionSet',
              selections: [
                { kind: 'Field', name: { kind: 'Name', value: 'amount' } },
                { kind: 'Field', name: { kind: 'Name', value: 'txHash' } },
                { kind: 'Field', name: { kind: 'Name', value: 'destNetwork' } },
                { kind: 'Field', name: { kind: 'Name', value: 'destination' } },
                { kind: 'Field', name: { kind: 'Name', value: 'id' } },
                { kind: 'Field', name: { kind: 'Name', value: 'receiver' } },
                { kind: 'Field', name: { kind: 'Name', value: 'sender' } },
                { kind: 'Field', name: { kind: 'Name', value: 'source' } },
                { kind: 'Field', name: { kind: 'Name', value: 'sourceNetwork' } },
                { kind: 'Field', name: { kind: 'Name', value: 'status' } },
                { kind: 'Field', name: { kind: 'Name', value: 'timestamp' } },
                { kind: 'Field', name: { kind: 'Name', value: 'nonce' } },
                { kind: 'Field', name: { kind: 'Name', value: 'blockNumber' } },
              ],
            },
          },
        ],
      },
    },
  ],
} as unknown as DocumentNode<TransfersQueryQuery, TransfersQueryQueryVariables>;
export const TransfersConnectionQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'TransfersConnectionQuery' },
      variableDefinitions: [
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'where' } },
          type: { kind: 'NamedType', name: { kind: 'Name', value: 'TransferWhereInput' } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'transfersConnection' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'orderBy' },
                value: { kind: 'EnumValue', value: 'timestamp_DESC' },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'where' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'where' } },
              },
            ],
            selectionSet: {
              kind: 'SelectionSet',
              selections: [{ kind: 'Field', name: { kind: 'Name', value: 'totalCount' } }],
            },
          },
        ],
      },
    },
  ],
} as unknown as DocumentNode<TransfersConnectionQueryQuery, TransfersConnectionQueryQueryVariables>;
export const PairsQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'PairsQuery' },
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'pairs' },
            selectionSet: {
              kind: 'SelectionSet',
              selections: [
                { kind: 'Field', name: { kind: 'Name', value: 'ethToken' } },
                { kind: 'Field', name: { kind: 'Name', value: 'ethTokenDecimals' } },
                { kind: 'Field', name: { kind: 'Name', value: 'ethTokenName' } },
                { kind: 'Field', name: { kind: 'Name', value: 'ethTokenSymbol' } },
                { kind: 'Field', name: { kind: 'Name', value: 'id' } },
                { kind: 'Field', name: { kind: 'Name', value: 'isRemoved' } },
                { kind: 'Field', name: { kind: 'Name', value: 'tokenSupply' } },
                { kind: 'Field', name: { kind: 'Name', value: 'varaToken' } },
                { kind: 'Field', name: { kind: 'Name', value: 'varaTokenDecimals' } },
                { kind: 'Field', name: { kind: 'Name', value: 'varaTokenName' } },
                { kind: 'Field', name: { kind: 'Name', value: 'varaTokenSymbol' } },
              ],
            },
          },
        ],
      },
    },
  ],
} as unknown as DocumentNode<PairsQueryQuery, PairsQueryQueryVariables>;
