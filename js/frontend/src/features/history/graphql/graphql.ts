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
  ID: { input: string; output: string };
  String: { input: string; output: string };
  Boolean: { input: boolean; output: boolean };
  Int: { input: number; output: number };
  Float: { input: number; output: number };
  /**
   * A signed eight-byte integer. The upper big integer values are greater than the
   * max value for a JavaScript number. Therefore all big integers will be output as
   * strings and not numbers.
   */
  BigInt: { input: string; output: string };
  /** A location in a connection that can be used for resuming pagination. */
  Cursor: { input: any; output: any };
  /**
   * A point in time as described by the [ISO
   * 8601](https://en.wikipedia.org/wiki/ISO_8601) standard. May or may not include a timezone.
   */
  Datetime: { input: string; output: string };
};

/** A filter to be used against BigInt fields. All fields are combined with a logical ‘and.’ */
export type BigIntFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<Scalars['BigInt']['input']>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<Scalars['BigInt']['input']>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<Scalars['BigInt']['input']>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<Scalars['BigInt']['input']>;
  /** Included in the specified list. */
  in: InputMaybe<Array<Scalars['BigInt']['input']>>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<Scalars['BigInt']['input']>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<Scalars['BigInt']['input']>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<Scalars['BigInt']['input']>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<Scalars['BigInt']['input']>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<Scalars['BigInt']['input']>>;
};

/** A filter to be used against Boolean fields. All fields are combined with a logical ‘and.’ */
export type BooleanFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<Scalars['Boolean']['input']>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<Scalars['Boolean']['input']>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<Scalars['Boolean']['input']>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<Scalars['Boolean']['input']>;
  /** Included in the specified list. */
  in: InputMaybe<Array<Scalars['Boolean']['input']>>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<Scalars['Boolean']['input']>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<Scalars['Boolean']['input']>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<Scalars['Boolean']['input']>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<Scalars['Boolean']['input']>>;
};

export type CheckpointSlot = Node & {
  __typename?: 'CheckpointSlot';
  id: Scalars['String']['output'];
  /** A globally unique identifier. Can be used in various places throughout the system to identify this single value. */
  nodeId: Scalars['ID']['output'];
  slot: Scalars['BigInt']['output'];
  treeHashRoot: Scalars['String']['output'];
};

/**
 * A condition to be used against `CheckpointSlot` object types. All fields are
 * tested for equality and combined with a logical ‘and.’
 */
export type CheckpointSlotCondition = {
  /** Checks for equality with the object’s `id` field. */
  id: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `slot` field. */
  slot: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `treeHashRoot` field. */
  treeHashRoot: InputMaybe<Scalars['String']['input']>;
};

/** A filter to be used against `CheckpointSlot` object types. All fields are combined with a logical ‘and.’ */
export type CheckpointSlotFilter = {
  /** Checks for all expressions in this list. */
  and: InputMaybe<Array<CheckpointSlotFilter>>;
  /** Filter by the object’s `id` field. */
  id: InputMaybe<StringFilter>;
  /** Negates the expression. */
  not: InputMaybe<CheckpointSlotFilter>;
  /** Checks for any expressions in this list. */
  or: InputMaybe<Array<CheckpointSlotFilter>>;
  /** Filter by the object’s `slot` field. */
  slot: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `treeHashRoot` field. */
  treeHashRoot: InputMaybe<StringFilter>;
};

/** A connection to a list of `CheckpointSlot` values. */
export type CheckpointSlotsConnection = {
  __typename?: 'CheckpointSlotsConnection';
  /** A list of edges which contains the `CheckpointSlot` and cursor to aid in pagination. */
  edges: Array<CheckpointSlotsEdge>;
  /** A list of `CheckpointSlot` objects. */
  nodes: Array<CheckpointSlot>;
  /** Information to aid in pagination. */
  pageInfo: PageInfo;
  /** The count of *all* `CheckpointSlot` you could get from the connection. */
  totalCount: Scalars['Int']['output'];
};

/** A `CheckpointSlot` edge in the connection. */
export type CheckpointSlotsEdge = {
  __typename?: 'CheckpointSlotsEdge';
  /** A cursor for use in pagination. */
  cursor: Maybe<Scalars['Cursor']['output']>;
  /** The `CheckpointSlot` at the end of the edge. */
  node: CheckpointSlot;
};

/** Methods to use when ordering `CheckpointSlot`. */
export enum CheckpointSlotsOrderBy {
  IdAsc = 'ID_ASC',
  IdDesc = 'ID_DESC',
  Natural = 'NATURAL',
  PrimaryKeyAsc = 'PRIMARY_KEY_ASC',
  PrimaryKeyDesc = 'PRIMARY_KEY_DESC',
  SlotAsc = 'SLOT_ASC',
  SlotDesc = 'SLOT_DESC',
  TreeHashRootAsc = 'TREE_HASH_ROOT_ASC',
  TreeHashRootDesc = 'TREE_HASH_ROOT_DESC',
}

/** A filter to be used against Datetime fields. All fields are combined with a logical ‘and.’ */
export type DatetimeFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<Scalars['Datetime']['input']>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<Scalars['Datetime']['input']>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<Scalars['Datetime']['input']>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<Scalars['Datetime']['input']>;
  /** Included in the specified list. */
  in: InputMaybe<Array<Scalars['Datetime']['input']>>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<Scalars['Datetime']['input']>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<Scalars['Datetime']['input']>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<Scalars['Datetime']['input']>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<Scalars['Datetime']['input']>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<Scalars['Datetime']['input']>>;
};

export type GearEthBridgeMessage = Node & {
  __typename?: 'GearEthBridgeMessage';
  blockNumber: Scalars['BigInt']['output'];
  id: Scalars['String']['output'];
  /** A globally unique identifier. Can be used in various places throughout the system to identify this single value. */
  nodeId: Scalars['ID']['output'];
  nonce: Scalars['String']['output'];
};

/**
 * A condition to be used against `GearEthBridgeMessage` object types. All fields
 * are tested for equality and combined with a logical ‘and.’
 */
export type GearEthBridgeMessageCondition = {
  /** Checks for equality with the object’s `blockNumber` field. */
  blockNumber: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `id` field. */
  id: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `nonce` field. */
  nonce: InputMaybe<Scalars['String']['input']>;
};

/** A filter to be used against `GearEthBridgeMessage` object types. All fields are combined with a logical ‘and.’ */
export type GearEthBridgeMessageFilter = {
  /** Checks for all expressions in this list. */
  and: InputMaybe<Array<GearEthBridgeMessageFilter>>;
  /** Filter by the object’s `blockNumber` field. */
  blockNumber: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `id` field. */
  id: InputMaybe<StringFilter>;
  /** Filter by the object’s `nonce` field. */
  nonce: InputMaybe<StringFilter>;
  /** Negates the expression. */
  not: InputMaybe<GearEthBridgeMessageFilter>;
  /** Checks for any expressions in this list. */
  or: InputMaybe<Array<GearEthBridgeMessageFilter>>;
};

/** A connection to a list of `GearEthBridgeMessage` values. */
export type GearEthBridgeMessagesConnection = {
  __typename?: 'GearEthBridgeMessagesConnection';
  /** A list of edges which contains the `GearEthBridgeMessage` and cursor to aid in pagination. */
  edges: Array<GearEthBridgeMessagesEdge>;
  /** A list of `GearEthBridgeMessage` objects. */
  nodes: Array<GearEthBridgeMessage>;
  /** Information to aid in pagination. */
  pageInfo: PageInfo;
  /** The count of *all* `GearEthBridgeMessage` you could get from the connection. */
  totalCount: Scalars['Int']['output'];
};

/** A `GearEthBridgeMessage` edge in the connection. */
export type GearEthBridgeMessagesEdge = {
  __typename?: 'GearEthBridgeMessagesEdge';
  /** A cursor for use in pagination. */
  cursor: Maybe<Scalars['Cursor']['output']>;
  /** The `GearEthBridgeMessage` at the end of the edge. */
  node: GearEthBridgeMessage;
};

/** Methods to use when ordering `GearEthBridgeMessage`. */
export enum GearEthBridgeMessagesOrderBy {
  BlockNumberAsc = 'BLOCK_NUMBER_ASC',
  BlockNumberDesc = 'BLOCK_NUMBER_DESC',
  IdAsc = 'ID_ASC',
  IdDesc = 'ID_DESC',
  Natural = 'NATURAL',
  NonceAsc = 'NONCE_ASC',
  NonceDesc = 'NONCE_DESC',
  PrimaryKeyAsc = 'PRIMARY_KEY_ASC',
  PrimaryKeyDesc = 'PRIMARY_KEY_DESC',
}

/** A filter to be used against Int fields. All fields are combined with a logical ‘and.’ */
export type IntFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<Scalars['Int']['input']>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<Scalars['Int']['input']>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<Scalars['Int']['input']>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<Scalars['Int']['input']>;
  /** Included in the specified list. */
  in: InputMaybe<Array<Scalars['Int']['input']>>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<Scalars['Int']['input']>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<Scalars['Int']['input']>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<Scalars['Int']['input']>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<Scalars['Int']['input']>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<Scalars['Int']['input']>>;
};

export type MerkleRootInMessageQueue = Node & {
  __typename?: 'MerkleRootInMessageQueue';
  blockNumber: Scalars['BigInt']['output'];
  id: Scalars['String']['output'];
  merkleRoot: Scalars['String']['output'];
  /** A globally unique identifier. Can be used in various places throughout the system to identify this single value. */
  nodeId: Scalars['ID']['output'];
};

/**
 * A condition to be used against `MerkleRootInMessageQueue` object types. All
 * fields are tested for equality and combined with a logical ‘and.’
 */
export type MerkleRootInMessageQueueCondition = {
  /** Checks for equality with the object’s `blockNumber` field. */
  blockNumber: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `id` field. */
  id: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `merkleRoot` field. */
  merkleRoot: InputMaybe<Scalars['String']['input']>;
};

/** A filter to be used against `MerkleRootInMessageQueue` object types. All fields are combined with a logical ‘and.’ */
export type MerkleRootInMessageQueueFilter = {
  /** Checks for all expressions in this list. */
  and: InputMaybe<Array<MerkleRootInMessageQueueFilter>>;
  /** Filter by the object’s `blockNumber` field. */
  blockNumber: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `id` field. */
  id: InputMaybe<StringFilter>;
  /** Filter by the object’s `merkleRoot` field. */
  merkleRoot: InputMaybe<StringFilter>;
  /** Negates the expression. */
  not: InputMaybe<MerkleRootInMessageQueueFilter>;
  /** Checks for any expressions in this list. */
  or: InputMaybe<Array<MerkleRootInMessageQueueFilter>>;
};

/** A connection to a list of `MerkleRootInMessageQueue` values. */
export type MerkleRootInMessageQueuesConnection = {
  __typename?: 'MerkleRootInMessageQueuesConnection';
  /** A list of edges which contains the `MerkleRootInMessageQueue` and cursor to aid in pagination. */
  edges: Array<MerkleRootInMessageQueuesEdge>;
  /** A list of `MerkleRootInMessageQueue` objects. */
  nodes: Array<MerkleRootInMessageQueue>;
  /** Information to aid in pagination. */
  pageInfo: PageInfo;
  /** The count of *all* `MerkleRootInMessageQueue` you could get from the connection. */
  totalCount: Scalars['Int']['output'];
};

/** A `MerkleRootInMessageQueue` edge in the connection. */
export type MerkleRootInMessageQueuesEdge = {
  __typename?: 'MerkleRootInMessageQueuesEdge';
  /** A cursor for use in pagination. */
  cursor: Maybe<Scalars['Cursor']['output']>;
  /** The `MerkleRootInMessageQueue` at the end of the edge. */
  node: MerkleRootInMessageQueue;
};

/** Methods to use when ordering `MerkleRootInMessageQueue`. */
export enum MerkleRootInMessageQueuesOrderBy {
  BlockNumberAsc = 'BLOCK_NUMBER_ASC',
  BlockNumberDesc = 'BLOCK_NUMBER_DESC',
  IdAsc = 'ID_ASC',
  IdDesc = 'ID_DESC',
  MerkleRootAsc = 'MERKLE_ROOT_ASC',
  MerkleRootDesc = 'MERKLE_ROOT_DESC',
  Natural = 'NATURAL',
  PrimaryKeyAsc = 'PRIMARY_KEY_ASC',
  PrimaryKeyDesc = 'PRIMARY_KEY_DESC',
}

export enum NetworkEnum {
  Ethereum = 'ETHEREUM',
  Vara = 'VARA',
}

/** A filter to be used against NetworkEnum fields. All fields are combined with a logical ‘and.’ */
export type NetworkEnumFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<NetworkEnum>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<NetworkEnum>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<NetworkEnum>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<NetworkEnum>;
  /** Included in the specified list. */
  in: InputMaybe<Array<NetworkEnum>>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<NetworkEnum>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<NetworkEnum>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<NetworkEnum>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<NetworkEnum>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<NetworkEnum>>;
};

/** An object with a globally unique `ID`. */
export type Node = {
  /** A globally unique identifier. Can be used in various places throughout the system to identify this single value. */
  nodeId: Scalars['ID']['output'];
};

/** Information about pagination in a connection. */
export type PageInfo = {
  __typename?: 'PageInfo';
  /** When paginating forwards, the cursor to continue. */
  endCursor: Maybe<Scalars['Cursor']['output']>;
  /** When paginating forwards, are there more items? */
  hasNextPage: Scalars['Boolean']['output'];
  /** When paginating backwards, are there more items? */
  hasPreviousPage: Scalars['Boolean']['output'];
  /** When paginating backwards, the cursor to continue. */
  startCursor: Maybe<Scalars['Cursor']['output']>;
};

export type Pair = Node & {
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
  /** A globally unique identifier. Can be used in various places throughout the system to identify this single value. */
  nodeId: Scalars['ID']['output'];
  tokenSupply: NetworkEnum;
  upgradedTo: Maybe<Scalars['String']['output']>;
  varaToken: Scalars['String']['output'];
  varaTokenDecimals: Scalars['Int']['output'];
  varaTokenName: Scalars['String']['output'];
  varaTokenSymbol: Scalars['String']['output'];
};

/** A condition to be used against `Pair` object types. All fields are tested for equality and combined with a logical ‘and.’ */
export type PairCondition = {
  /** Checks for equality with the object’s `activeSinceBlock` field. */
  activeSinceBlock: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `activeToBlock` field. */
  activeToBlock: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `ethToken` field. */
  ethToken: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `ethTokenDecimals` field. */
  ethTokenDecimals: InputMaybe<Scalars['Int']['input']>;
  /** Checks for equality with the object’s `ethTokenName` field. */
  ethTokenName: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `ethTokenSymbol` field. */
  ethTokenSymbol: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `id` field. */
  id: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `isActive` field. */
  isActive: InputMaybe<Scalars['Boolean']['input']>;
  /** Checks for equality with the object’s `isRemoved` field. */
  isRemoved: InputMaybe<Scalars['Boolean']['input']>;
  /** Checks for equality with the object’s `tokenSupply` field. */
  tokenSupply: InputMaybe<NetworkEnum>;
  /** Checks for equality with the object’s `upgradedTo` field. */
  upgradedTo: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `varaToken` field. */
  varaToken: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `varaTokenDecimals` field. */
  varaTokenDecimals: InputMaybe<Scalars['Int']['input']>;
  /** Checks for equality with the object’s `varaTokenName` field. */
  varaTokenName: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `varaTokenSymbol` field. */
  varaTokenSymbol: InputMaybe<Scalars['String']['input']>;
};

/** A filter to be used against `Pair` object types. All fields are combined with a logical ‘and.’ */
export type PairFilter = {
  /** Filter by the object’s `activeSinceBlock` field. */
  activeSinceBlock: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `activeToBlock` field. */
  activeToBlock: InputMaybe<BigIntFilter>;
  /** Checks for all expressions in this list. */
  and: InputMaybe<Array<PairFilter>>;
  /** Filter by the object’s `ethToken` field. */
  ethToken: InputMaybe<StringFilter>;
  /** Filter by the object’s `ethTokenDecimals` field. */
  ethTokenDecimals: InputMaybe<IntFilter>;
  /** Filter by the object’s `ethTokenName` field. */
  ethTokenName: InputMaybe<StringFilter>;
  /** Filter by the object’s `ethTokenSymbol` field. */
  ethTokenSymbol: InputMaybe<StringFilter>;
  /** Filter by the object’s `id` field. */
  id: InputMaybe<StringFilter>;
  /** Filter by the object’s `isActive` field. */
  isActive: InputMaybe<BooleanFilter>;
  /** Filter by the object’s `isRemoved` field. */
  isRemoved: InputMaybe<BooleanFilter>;
  /** Negates the expression. */
  not: InputMaybe<PairFilter>;
  /** Checks for any expressions in this list. */
  or: InputMaybe<Array<PairFilter>>;
  /** Filter by the object’s `tokenSupply` field. */
  tokenSupply: InputMaybe<NetworkEnumFilter>;
  /** Filter by the object’s `upgradedTo` field. */
  upgradedTo: InputMaybe<StringFilter>;
  /** Filter by the object’s `varaToken` field. */
  varaToken: InputMaybe<StringFilter>;
  /** Filter by the object’s `varaTokenDecimals` field. */
  varaTokenDecimals: InputMaybe<IntFilter>;
  /** Filter by the object’s `varaTokenName` field. */
  varaTokenName: InputMaybe<StringFilter>;
  /** Filter by the object’s `varaTokenSymbol` field. */
  varaTokenSymbol: InputMaybe<StringFilter>;
};

/** A connection to a list of `Pair` values. */
export type PairsConnection = {
  __typename?: 'PairsConnection';
  /** A list of edges which contains the `Pair` and cursor to aid in pagination. */
  edges: Array<PairsEdge>;
  /** A list of `Pair` objects. */
  nodes: Array<Pair>;
  /** Information to aid in pagination. */
  pageInfo: PageInfo;
  /** The count of *all* `Pair` you could get from the connection. */
  totalCount: Scalars['Int']['output'];
};

/** A `Pair` edge in the connection. */
export type PairsEdge = {
  __typename?: 'PairsEdge';
  /** A cursor for use in pagination. */
  cursor: Maybe<Scalars['Cursor']['output']>;
  /** The `Pair` at the end of the edge. */
  node: Pair;
};

/** Methods to use when ordering `Pair`. */
export enum PairsOrderBy {
  ActiveSinceBlockAsc = 'ACTIVE_SINCE_BLOCK_ASC',
  ActiveSinceBlockDesc = 'ACTIVE_SINCE_BLOCK_DESC',
  ActiveToBlockAsc = 'ACTIVE_TO_BLOCK_ASC',
  ActiveToBlockDesc = 'ACTIVE_TO_BLOCK_DESC',
  EthTokenAsc = 'ETH_TOKEN_ASC',
  EthTokenDecimalsAsc = 'ETH_TOKEN_DECIMALS_ASC',
  EthTokenDecimalsDesc = 'ETH_TOKEN_DECIMALS_DESC',
  EthTokenDesc = 'ETH_TOKEN_DESC',
  EthTokenNameAsc = 'ETH_TOKEN_NAME_ASC',
  EthTokenNameDesc = 'ETH_TOKEN_NAME_DESC',
  EthTokenSymbolAsc = 'ETH_TOKEN_SYMBOL_ASC',
  EthTokenSymbolDesc = 'ETH_TOKEN_SYMBOL_DESC',
  IdAsc = 'ID_ASC',
  IdDesc = 'ID_DESC',
  IsActiveAsc = 'IS_ACTIVE_ASC',
  IsActiveDesc = 'IS_ACTIVE_DESC',
  IsRemovedAsc = 'IS_REMOVED_ASC',
  IsRemovedDesc = 'IS_REMOVED_DESC',
  Natural = 'NATURAL',
  PrimaryKeyAsc = 'PRIMARY_KEY_ASC',
  PrimaryKeyDesc = 'PRIMARY_KEY_DESC',
  TokenSupplyAsc = 'TOKEN_SUPPLY_ASC',
  TokenSupplyDesc = 'TOKEN_SUPPLY_DESC',
  UpgradedToAsc = 'UPGRADED_TO_ASC',
  UpgradedToDesc = 'UPGRADED_TO_DESC',
  VaraTokenAsc = 'VARA_TOKEN_ASC',
  VaraTokenDecimalsAsc = 'VARA_TOKEN_DECIMALS_ASC',
  VaraTokenDecimalsDesc = 'VARA_TOKEN_DECIMALS_DESC',
  VaraTokenDesc = 'VARA_TOKEN_DESC',
  VaraTokenNameAsc = 'VARA_TOKEN_NAME_ASC',
  VaraTokenNameDesc = 'VARA_TOKEN_NAME_DESC',
  VaraTokenSymbolAsc = 'VARA_TOKEN_SYMBOL_ASC',
  VaraTokenSymbolDesc = 'VARA_TOKEN_SYMBOL_DESC',
}

/** The root query type which gives access points into the data universe. */
export type Query = Node & {
  __typename?: 'Query';
  /** Reads and enables pagination through a set of `CheckpointSlot`. */
  allCheckpointSlots: Maybe<CheckpointSlotsConnection>;
  /** Reads and enables pagination through a set of `GearEthBridgeMessage`. */
  allGearEthBridgeMessages: Maybe<GearEthBridgeMessagesConnection>;
  /** Reads and enables pagination through a set of `MerkleRootInMessageQueue`. */
  allMerkleRootInMessageQueues: Maybe<MerkleRootInMessageQueuesConnection>;
  /** Reads and enables pagination through a set of `Pair`. */
  allPairs: Maybe<PairsConnection>;
  /** Reads and enables pagination through a set of `Transfer`. */
  allTransfers: Maybe<TransfersConnection>;
  /** Reads a single `CheckpointSlot` using its globally unique `ID`. */
  checkpointSlot: Maybe<CheckpointSlot>;
  checkpointSlotById: Maybe<CheckpointSlot>;
  /** Reads a single `GearEthBridgeMessage` using its globally unique `ID`. */
  gearEthBridgeMessage: Maybe<GearEthBridgeMessage>;
  gearEthBridgeMessageById: Maybe<GearEthBridgeMessage>;
  /** Reads a single `MerkleRootInMessageQueue` using its globally unique `ID`. */
  merkleRootInMessageQueue: Maybe<MerkleRootInMessageQueue>;
  merkleRootInMessageQueueByBlockNumber: Maybe<MerkleRootInMessageQueue>;
  merkleRootInMessageQueueById: Maybe<MerkleRootInMessageQueue>;
  /** Fetches an object given its globally unique `ID`. */
  node: Maybe<Node>;
  /** The root query type must be a `Node` to work well with Relay 1 mutations. This just resolves to `query`. */
  nodeId: Scalars['ID']['output'];
  /** Reads a single `Pair` using its globally unique `ID`. */
  pair: Maybe<Pair>;
  pairById: Maybe<Pair>;
  pairs: Array<Maybe<Pair>>;
  /**
   * Exposes the root query type nested one level down. This is helpful for Relay 1
   * which can only query top level fields if they are in a particular form.
   */
  query: Query;
  /** Reads a single `Transfer` using its globally unique `ID`. */
  transfer: Maybe<Transfer>;
  transferById: Maybe<Transfer>;
  transfers: Array<Maybe<Transfer>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryAllCheckpointSlotsArgs = {
  after: InputMaybe<Scalars['Cursor']['input']>;
  before: InputMaybe<Scalars['Cursor']['input']>;
  condition: InputMaybe<CheckpointSlotCondition>;
  filter: InputMaybe<CheckpointSlotFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy?: InputMaybe<Array<CheckpointSlotsOrderBy>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryAllGearEthBridgeMessagesArgs = {
  after: InputMaybe<Scalars['Cursor']['input']>;
  before: InputMaybe<Scalars['Cursor']['input']>;
  condition: InputMaybe<GearEthBridgeMessageCondition>;
  filter: InputMaybe<GearEthBridgeMessageFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy?: InputMaybe<Array<GearEthBridgeMessagesOrderBy>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryAllMerkleRootInMessageQueuesArgs = {
  after: InputMaybe<Scalars['Cursor']['input']>;
  before: InputMaybe<Scalars['Cursor']['input']>;
  condition: InputMaybe<MerkleRootInMessageQueueCondition>;
  filter: InputMaybe<MerkleRootInMessageQueueFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy?: InputMaybe<Array<MerkleRootInMessageQueuesOrderBy>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryAllPairsArgs = {
  after: InputMaybe<Scalars['Cursor']['input']>;
  before: InputMaybe<Scalars['Cursor']['input']>;
  condition: InputMaybe<PairCondition>;
  filter: InputMaybe<PairFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy?: InputMaybe<Array<PairsOrderBy>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryAllTransfersArgs = {
  after: InputMaybe<Scalars['Cursor']['input']>;
  before: InputMaybe<Scalars['Cursor']['input']>;
  condition: InputMaybe<TransferCondition>;
  filter: InputMaybe<TransferFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy?: InputMaybe<Array<TransfersOrderBy>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryCheckpointSlotArgs = {
  nodeId: Scalars['ID']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryCheckpointSlotByIdArgs = {
  id: Scalars['String']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryGearEthBridgeMessageArgs = {
  nodeId: Scalars['ID']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryGearEthBridgeMessageByIdArgs = {
  id: Scalars['String']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryMerkleRootInMessageQueueArgs = {
  nodeId: Scalars['ID']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryMerkleRootInMessageQueueByBlockNumberArgs = {
  blockNumber: Scalars['BigInt']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryMerkleRootInMessageQueueByIdArgs = {
  id: Scalars['String']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryNodeArgs = {
  nodeId: Scalars['ID']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryPairArgs = {
  nodeId: Scalars['ID']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryPairByIdArgs = {
  id: Scalars['String']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryPairsArgs = {
  filter: InputMaybe<PairFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<PairsOrderBy>>;
};

/** The root query type which gives access points into the data universe. */
export type QueryTransferArgs = {
  nodeId: Scalars['ID']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryTransferByIdArgs = {
  id: Scalars['String']['input'];
};

/** The root query type which gives access points into the data universe. */
export type QueryTransfersArgs = {
  filter: InputMaybe<TransferFilter>;
  first: InputMaybe<Scalars['Int']['input']>;
  last: InputMaybe<Scalars['Int']['input']>;
  offset: InputMaybe<Scalars['Int']['input']>;
  orderBy: InputMaybe<Array<TransfersOrderBy>>;
};

export enum StatusEnum {
  AwaitingPayment = 'AWAITING_PAYMENT',
  Bridging = 'BRIDGING',
  Completed = 'COMPLETED',
  Failed = 'FAILED',
}

/** A filter to be used against StatusEnum fields. All fields are combined with a logical ‘and.’ */
export type StatusEnumFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<StatusEnum>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<StatusEnum>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<StatusEnum>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<StatusEnum>;
  /** Included in the specified list. */
  in: InputMaybe<Array<StatusEnum>>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<StatusEnum>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<StatusEnum>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<StatusEnum>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<StatusEnum>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<StatusEnum>>;
};

/** A filter to be used against String fields. All fields are combined with a logical ‘and.’ */
export type StringFilter = {
  /** Not equal to the specified value, treating null like an ordinary value. */
  distinctFrom: InputMaybe<Scalars['String']['input']>;
  /** Not equal to the specified value, treating null like an ordinary value (case-insensitive). */
  distinctFromInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Ends with the specified string (case-sensitive). */
  endsWith: InputMaybe<Scalars['String']['input']>;
  /** Ends with the specified string (case-insensitive). */
  endsWithInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Equal to the specified value. */
  equalTo: InputMaybe<Scalars['String']['input']>;
  /** Equal to the specified value (case-insensitive). */
  equalToInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Greater than the specified value. */
  greaterThan: InputMaybe<Scalars['String']['input']>;
  /** Greater than the specified value (case-insensitive). */
  greaterThanInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Greater than or equal to the specified value. */
  greaterThanOrEqualTo: InputMaybe<Scalars['String']['input']>;
  /** Greater than or equal to the specified value (case-insensitive). */
  greaterThanOrEqualToInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Included in the specified list. */
  in: InputMaybe<Array<Scalars['String']['input']>>;
  /** Included in the specified list (case-insensitive). */
  inInsensitive: InputMaybe<Array<Scalars['String']['input']>>;
  /** Contains the specified string (case-sensitive). */
  includes: InputMaybe<Scalars['String']['input']>;
  /** Contains the specified string (case-insensitive). */
  includesInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Is null (if `true` is specified) or is not null (if `false` is specified). */
  isNull: InputMaybe<Scalars['Boolean']['input']>;
  /** Less than the specified value. */
  lessThan: InputMaybe<Scalars['String']['input']>;
  /** Less than the specified value (case-insensitive). */
  lessThanInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Less than or equal to the specified value. */
  lessThanOrEqualTo: InputMaybe<Scalars['String']['input']>;
  /** Less than or equal to the specified value (case-insensitive). */
  lessThanOrEqualToInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Matches the specified pattern (case-sensitive). An underscore (_) matches any single character; a percent sign (%) matches any sequence of zero or more characters. */
  like: InputMaybe<Scalars['String']['input']>;
  /** Matches the specified pattern (case-insensitive). An underscore (_) matches any single character; a percent sign (%) matches any sequence of zero or more characters. */
  likeInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Equal to the specified value, treating null like an ordinary value. */
  notDistinctFrom: InputMaybe<Scalars['String']['input']>;
  /** Equal to the specified value, treating null like an ordinary value (case-insensitive). */
  notDistinctFromInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Does not end with the specified string (case-sensitive). */
  notEndsWith: InputMaybe<Scalars['String']['input']>;
  /** Does not end with the specified string (case-insensitive). */
  notEndsWithInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Not equal to the specified value. */
  notEqualTo: InputMaybe<Scalars['String']['input']>;
  /** Not equal to the specified value (case-insensitive). */
  notEqualToInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Not included in the specified list. */
  notIn: InputMaybe<Array<Scalars['String']['input']>>;
  /** Not included in the specified list (case-insensitive). */
  notInInsensitive: InputMaybe<Array<Scalars['String']['input']>>;
  /** Does not contain the specified string (case-sensitive). */
  notIncludes: InputMaybe<Scalars['String']['input']>;
  /** Does not contain the specified string (case-insensitive). */
  notIncludesInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Does not match the specified pattern (case-sensitive). An underscore (_) matches any single character; a percent sign (%) matches any sequence of zero or more characters. */
  notLike: InputMaybe<Scalars['String']['input']>;
  /** Does not match the specified pattern (case-insensitive). An underscore (_) matches any single character; a percent sign (%) matches any sequence of zero or more characters. */
  notLikeInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Does not start with the specified string (case-sensitive). */
  notStartsWith: InputMaybe<Scalars['String']['input']>;
  /** Does not start with the specified string (case-insensitive). */
  notStartsWithInsensitive: InputMaybe<Scalars['String']['input']>;
  /** Starts with the specified string (case-sensitive). */
  startsWith: InputMaybe<Scalars['String']['input']>;
  /** Starts with the specified string (case-insensitive). */
  startsWithInsensitive: InputMaybe<Scalars['String']['input']>;
};

/** The root subscription type: contains realtime events you can subscribe to with the `subscription` operation. */
export type Subscription = {
  __typename?: 'Subscription';
  transferCount: Scalars['Int']['output'];
};

export type Transfer = Node & {
  __typename?: 'Transfer';
  amount: Scalars['String']['output'];
  blockNumber: Scalars['BigInt']['output'];
  bridgingStartedAtBlock: Maybe<Scalars['BigInt']['output']>;
  bridgingStartedAtMessageId: Maybe<Scalars['String']['output']>;
  completedAt: Maybe<Scalars['Datetime']['output']>;
  completedAtBlock: Maybe<Scalars['BigInt']['output']>;
  completedAtTxHash: Maybe<Scalars['String']['output']>;
  destNetwork: NetworkEnum;
  destination: Scalars['String']['output'];
  id: Scalars['String']['output'];
  /** A globally unique identifier. Can be used in various places throughout the system to identify this single value. */
  nodeId: Scalars['ID']['output'];
  nonce: Scalars['String']['output'];
  receiver: Scalars['String']['output'];
  sender: Scalars['String']['output'];
  source: Scalars['String']['output'];
  sourceNetwork: NetworkEnum;
  status: StatusEnum;
  timestamp: Scalars['Datetime']['output'];
  txHash: Scalars['String']['output'];
};

/**
 * A condition to be used against `Transfer` object types. All fields are tested
 * for equality and combined with a logical ‘and.’
 */
export type TransferCondition = {
  /** Checks for equality with the object’s `amount` field. */
  amount: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `blockNumber` field. */
  blockNumber: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `bridgingStartedAtBlock` field. */
  bridgingStartedAtBlock: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `bridgingStartedAtMessageId` field. */
  bridgingStartedAtMessageId: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `completedAt` field. */
  completedAt: InputMaybe<Scalars['Datetime']['input']>;
  /** Checks for equality with the object’s `completedAtBlock` field. */
  completedAtBlock: InputMaybe<Scalars['BigInt']['input']>;
  /** Checks for equality with the object’s `completedAtTxHash` field. */
  completedAtTxHash: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `destNetwork` field. */
  destNetwork: InputMaybe<NetworkEnum>;
  /** Checks for equality with the object’s `destination` field. */
  destination: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `id` field. */
  id: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `nonce` field. */
  nonce: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `receiver` field. */
  receiver: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `sender` field. */
  sender: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `source` field. */
  source: InputMaybe<Scalars['String']['input']>;
  /** Checks for equality with the object’s `sourceNetwork` field. */
  sourceNetwork: InputMaybe<NetworkEnum>;
  /** Checks for equality with the object’s `status` field. */
  status: InputMaybe<StatusEnum>;
  /** Checks for equality with the object’s `timestamp` field. */
  timestamp: InputMaybe<Scalars['Datetime']['input']>;
  /** Checks for equality with the object’s `txHash` field. */
  txHash: InputMaybe<Scalars['String']['input']>;
};

/** A filter to be used against `Transfer` object types. All fields are combined with a logical ‘and.’ */
export type TransferFilter = {
  /** Filter by the object’s `amount` field. */
  amount: InputMaybe<StringFilter>;
  /** Checks for all expressions in this list. */
  and: InputMaybe<Array<TransferFilter>>;
  /** Filter by the object’s `blockNumber` field. */
  blockNumber: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `bridgingStartedAtBlock` field. */
  bridgingStartedAtBlock: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `bridgingStartedAtMessageId` field. */
  bridgingStartedAtMessageId: InputMaybe<StringFilter>;
  /** Filter by the object’s `completedAt` field. */
  completedAt: InputMaybe<DatetimeFilter>;
  /** Filter by the object’s `completedAtBlock` field. */
  completedAtBlock: InputMaybe<BigIntFilter>;
  /** Filter by the object’s `completedAtTxHash` field. */
  completedAtTxHash: InputMaybe<StringFilter>;
  /** Filter by the object’s `destNetwork` field. */
  destNetwork: InputMaybe<NetworkEnumFilter>;
  /** Filter by the object’s `destination` field. */
  destination: InputMaybe<StringFilter>;
  /** Filter by the object’s `id` field. */
  id: InputMaybe<StringFilter>;
  /** Filter by the object’s `nonce` field. */
  nonce: InputMaybe<StringFilter>;
  /** Negates the expression. */
  not: InputMaybe<TransferFilter>;
  /** Checks for any expressions in this list. */
  or: InputMaybe<Array<TransferFilter>>;
  /** Filter by the object’s `receiver` field. */
  receiver: InputMaybe<StringFilter>;
  /** Filter by the object’s `sender` field. */
  sender: InputMaybe<StringFilter>;
  /** Filter by the object’s `source` field. */
  source: InputMaybe<StringFilter>;
  /** Filter by the object’s `sourceNetwork` field. */
  sourceNetwork: InputMaybe<NetworkEnumFilter>;
  /** Filter by the object’s `status` field. */
  status: InputMaybe<StatusEnumFilter>;
  /** Filter by the object’s `timestamp` field. */
  timestamp: InputMaybe<DatetimeFilter>;
  /** Filter by the object’s `txHash` field. */
  txHash: InputMaybe<StringFilter>;
};

/** A connection to a list of `Transfer` values. */
export type TransfersConnection = {
  __typename?: 'TransfersConnection';
  /** A list of edges which contains the `Transfer` and cursor to aid in pagination. */
  edges: Array<TransfersEdge>;
  /** A list of `Transfer` objects. */
  nodes: Array<Transfer>;
  /** Information to aid in pagination. */
  pageInfo: PageInfo;
  /** The count of *all* `Transfer` you could get from the connection. */
  totalCount: Scalars['Int']['output'];
};

/** A `Transfer` edge in the connection. */
export type TransfersEdge = {
  __typename?: 'TransfersEdge';
  /** A cursor for use in pagination. */
  cursor: Maybe<Scalars['Cursor']['output']>;
  /** The `Transfer` at the end of the edge. */
  node: Transfer;
};

/** Methods to use when ordering `Transfer`. */
export enum TransfersOrderBy {
  AmountAsc = 'AMOUNT_ASC',
  AmountDesc = 'AMOUNT_DESC',
  BlockNumberAsc = 'BLOCK_NUMBER_ASC',
  BlockNumberDesc = 'BLOCK_NUMBER_DESC',
  BridgingStartedAtBlockAsc = 'BRIDGING_STARTED_AT_BLOCK_ASC',
  BridgingStartedAtBlockDesc = 'BRIDGING_STARTED_AT_BLOCK_DESC',
  BridgingStartedAtMessageIdAsc = 'BRIDGING_STARTED_AT_MESSAGE_ID_ASC',
  BridgingStartedAtMessageIdDesc = 'BRIDGING_STARTED_AT_MESSAGE_ID_DESC',
  CompletedAtAsc = 'COMPLETED_AT_ASC',
  CompletedAtBlockAsc = 'COMPLETED_AT_BLOCK_ASC',
  CompletedAtBlockDesc = 'COMPLETED_AT_BLOCK_DESC',
  CompletedAtDesc = 'COMPLETED_AT_DESC',
  CompletedAtTxHashAsc = 'COMPLETED_AT_TX_HASH_ASC',
  CompletedAtTxHashDesc = 'COMPLETED_AT_TX_HASH_DESC',
  DestinationAsc = 'DESTINATION_ASC',
  DestinationDesc = 'DESTINATION_DESC',
  DestNetworkAsc = 'DEST_NETWORK_ASC',
  DestNetworkDesc = 'DEST_NETWORK_DESC',
  IdAsc = 'ID_ASC',
  IdDesc = 'ID_DESC',
  Natural = 'NATURAL',
  NonceAsc = 'NONCE_ASC',
  NonceDesc = 'NONCE_DESC',
  PrimaryKeyAsc = 'PRIMARY_KEY_ASC',
  PrimaryKeyDesc = 'PRIMARY_KEY_DESC',
  ReceiverAsc = 'RECEIVER_ASC',
  ReceiverDesc = 'RECEIVER_DESC',
  SenderAsc = 'SENDER_ASC',
  SenderDesc = 'SENDER_DESC',
  SourceAsc = 'SOURCE_ASC',
  SourceDesc = 'SOURCE_DESC',
  SourceNetworkAsc = 'SOURCE_NETWORK_ASC',
  SourceNetworkDesc = 'SOURCE_NETWORK_DESC',
  StatusAsc = 'STATUS_ASC',
  StatusDesc = 'STATUS_DESC',
  TimestampAsc = 'TIMESTAMP_ASC',
  TimestampDesc = 'TIMESTAMP_DESC',
  TxHashAsc = 'TX_HASH_ASC',
  TxHashDesc = 'TX_HASH_DESC',
}

export type TransfersQueryQueryVariables = Exact<{
  first: Scalars['Int']['input'];
  offset: Scalars['Int']['input'];
  filter: InputMaybe<TransferFilter>;
}>;

export type TransfersQueryQuery = {
  __typename?: 'Query';
  allTransfers: {
    __typename?: 'TransfersConnection';
    totalCount: number;
    nodes: Array<{
      __typename?: 'Transfer';
      amount: string;
      txHash: string;
      destNetwork: NetworkEnum;
      destination: string;
      id: string;
      receiver: string;
      sender: string;
      source: string;
      sourceNetwork: NetworkEnum;
      status: StatusEnum;
      timestamp: string;
      nonce: string;
      blockNumber: string;
    }>;
  } | null;
};

export type PairsQueryQueryVariables = Exact<{ [key: string]: never }>;

export type PairsQueryQuery = {
  __typename?: 'Query';
  allPairs: {
    __typename?: 'PairsConnection';
    nodes: Array<{
      __typename?: 'Pair';
      ethToken: string;
      ethTokenDecimals: number;
      ethTokenName: string;
      ethTokenSymbol: string;
      id: string;
      isActive: boolean;
      tokenSupply: NetworkEnum;
      varaToken: string;
      varaTokenDecimals: number;
      varaTokenName: string;
      varaTokenSymbol: string;
    }>;
  } | null;
};

export type TransferQueryQueryVariables = Exact<{
  id: Scalars['String']['input'];
}>;

export type TransferQueryQuery = {
  __typename?: 'Query';
  transferById: {
    __typename?: 'Transfer';
    id: string;
    txHash: string;
    blockNumber: string;
    timestamp: string;
    completedAt: string | null;
    completedAtBlock: string | null;
    completedAtTxHash: string | null;
    nonce: string;
    sourceNetwork: NetworkEnum;
    source: string;
    destNetwork: NetworkEnum;
    destination: string;
    status: StatusEnum;
    sender: string;
    receiver: string;
    amount: string;
    bridgingStartedAtBlock: string | null;
    bridgingStartedAtMessageId: string | null;
  } | null;
};

export type TransfersCountQueryQueryVariables = Exact<{
  filter: InputMaybe<TransferFilter>;
}>;

export type TransfersCountQueryQuery = {
  __typename?: 'Query';
  allTransfers: { __typename?: 'TransfersConnection'; totalCount: number } | null;
};

export type CheckpointSlotsQueryQueryVariables = Exact<{
  slot: Scalars['BigInt']['input'];
}>;

export type CheckpointSlotsQueryQuery = {
  __typename?: 'Query';
  allCheckpointSlots: { __typename?: 'CheckpointSlotsConnection'; totalCount: number } | null;
};

export type MerkelRootInMessageQueuesQueryQueryVariables = Exact<{
  blockNumber: Scalars['BigInt']['input'];
}>;

export type MerkelRootInMessageQueuesQueryQuery = {
  __typename?: 'Query';
  allMerkleRootInMessageQueues: { __typename?: 'MerkleRootInMessageQueuesConnection'; totalCount: number } | null;
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
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'first' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } } },
        },
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'offset' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } } },
        },
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
          type: { kind: 'NamedType', name: { kind: 'Name', value: 'TransferFilter' } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'allTransfers' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'first' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'first' } },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'offset' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'offset' } },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'orderBy' },
                value: { kind: 'EnumValue', value: 'TIMESTAMP_DESC' },
              },
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'filter' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
              },
            ],
            selectionSet: {
              kind: 'SelectionSet',
              selections: [
                {
                  kind: 'Field',
                  name: { kind: 'Name', value: 'nodes' },
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
                { kind: 'Field', name: { kind: 'Name', value: 'totalCount' } },
              ],
            },
          },
        ],
      },
    },
  ],
} as unknown as DocumentNode<TransfersQueryQuery, TransfersQueryQueryVariables>;
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
            name: { kind: 'Name', value: 'allPairs' },
            selectionSet: {
              kind: 'SelectionSet',
              selections: [
                {
                  kind: 'Field',
                  name: { kind: 'Name', value: 'nodes' },
                  selectionSet: {
                    kind: 'SelectionSet',
                    selections: [
                      { kind: 'Field', name: { kind: 'Name', value: 'ethToken' } },
                      { kind: 'Field', name: { kind: 'Name', value: 'ethTokenDecimals' } },
                      { kind: 'Field', name: { kind: 'Name', value: 'ethTokenName' } },
                      { kind: 'Field', name: { kind: 'Name', value: 'ethTokenSymbol' } },
                      { kind: 'Field', name: { kind: 'Name', value: 'id' } },
                      { kind: 'Field', name: { kind: 'Name', value: 'isActive' } },
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
      },
    },
  ],
} as unknown as DocumentNode<PairsQueryQuery, PairsQueryQueryVariables>;
export const TransferQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'TransferQuery' },
      variableDefinitions: [
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'transferById' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'id' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
              },
            ],
            selectionSet: {
              kind: 'SelectionSet',
              selections: [
                { kind: 'Field', name: { kind: 'Name', value: 'id' } },
                { kind: 'Field', name: { kind: 'Name', value: 'txHash' } },
                { kind: 'Field', name: { kind: 'Name', value: 'blockNumber' } },
                { kind: 'Field', name: { kind: 'Name', value: 'timestamp' } },
                { kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
                { kind: 'Field', name: { kind: 'Name', value: 'completedAtBlock' } },
                { kind: 'Field', name: { kind: 'Name', value: 'completedAtTxHash' } },
                { kind: 'Field', name: { kind: 'Name', value: 'nonce' } },
                { kind: 'Field', name: { kind: 'Name', value: 'sourceNetwork' } },
                { kind: 'Field', name: { kind: 'Name', value: 'source' } },
                { kind: 'Field', name: { kind: 'Name', value: 'destNetwork' } },
                { kind: 'Field', name: { kind: 'Name', value: 'destination' } },
                { kind: 'Field', name: { kind: 'Name', value: 'status' } },
                { kind: 'Field', name: { kind: 'Name', value: 'sender' } },
                { kind: 'Field', name: { kind: 'Name', value: 'receiver' } },
                { kind: 'Field', name: { kind: 'Name', value: 'amount' } },
                { kind: 'Field', name: { kind: 'Name', value: 'bridgingStartedAtBlock' } },
                { kind: 'Field', name: { kind: 'Name', value: 'bridgingStartedAtMessageId' } },
              ],
            },
          },
        ],
      },
    },
  ],
} as unknown as DocumentNode<TransferQueryQuery, TransferQueryQueryVariables>;
export const TransfersCountQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'TransfersCountQuery' },
      variableDefinitions: [
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
          type: { kind: 'NamedType', name: { kind: 'Name', value: 'TransferFilter' } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'allTransfers' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'filter' },
                value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
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
} as unknown as DocumentNode<TransfersCountQueryQuery, TransfersCountQueryQueryVariables>;
export const CheckpointSlotsQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'CheckpointSlotsQuery' },
      variableDefinitions: [
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'slot' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'BigInt' } } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'allCheckpointSlots' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'filter' },
                value: {
                  kind: 'ObjectValue',
                  fields: [
                    {
                      kind: 'ObjectField',
                      name: { kind: 'Name', value: 'slot' },
                      value: {
                        kind: 'ObjectValue',
                        fields: [
                          {
                            kind: 'ObjectField',
                            name: { kind: 'Name', value: 'greaterThanOrEqualTo' },
                            value: { kind: 'Variable', name: { kind: 'Name', value: 'slot' } },
                          },
                        ],
                      },
                    },
                  ],
                },
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
} as unknown as DocumentNode<CheckpointSlotsQueryQuery, CheckpointSlotsQueryQueryVariables>;
export const MerkelRootInMessageQueuesQueryDocument = {
  kind: 'Document',
  definitions: [
    {
      kind: 'OperationDefinition',
      operation: 'query',
      name: { kind: 'Name', value: 'MerkelRootInMessageQueuesQuery' },
      variableDefinitions: [
        {
          kind: 'VariableDefinition',
          variable: { kind: 'Variable', name: { kind: 'Name', value: 'blockNumber' } },
          type: { kind: 'NonNullType', type: { kind: 'NamedType', name: { kind: 'Name', value: 'BigInt' } } },
        },
      ],
      selectionSet: {
        kind: 'SelectionSet',
        selections: [
          {
            kind: 'Field',
            name: { kind: 'Name', value: 'allMerkleRootInMessageQueues' },
            arguments: [
              {
                kind: 'Argument',
                name: { kind: 'Name', value: 'filter' },
                value: {
                  kind: 'ObjectValue',
                  fields: [
                    {
                      kind: 'ObjectField',
                      name: { kind: 'Name', value: 'blockNumber' },
                      value: {
                        kind: 'ObjectValue',
                        fields: [
                          {
                            kind: 'ObjectField',
                            name: { kind: 'Name', value: 'greaterThanOrEqualTo' },
                            value: { kind: 'Variable', name: { kind: 'Name', value: 'blockNumber' } },
                          },
                        ],
                      },
                    },
                  ],
                },
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
} as unknown as DocumentNode<MerkelRootInMessageQueuesQueryQuery, MerkelRootInMessageQueuesQueryQueryVariables>;
