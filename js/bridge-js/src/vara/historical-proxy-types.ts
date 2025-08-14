export const HistoricalProxyTypes = {
  EthToVaraEvent: {
    proofBlock: 'BlockInclusionProof',
    proof: 'Vec<Vec<u8>>',
    transactionIndex: 'u64',
    receiptRlp: 'Vec<u8>',
  },
  BlockInclusionProof: {
    block: 'BlockGenericForBlockBody',
    headers: 'Vec<BlockHeader>',
  },
  BlockHeader: {
    slot: 'u64',
    proposerIndex: 'u64',
    parentRoot: 'H256',
    stateRoot: 'H256',
    bodyRoot: 'H256',
  },
  BlockGenericForBlockBody: {
    slot: 'u64',
    proposerIndex: 'u64',
    parentRoot: 'H256',
    stateRoot: 'H256',
    body: 'BlockBody',
  },
  BlockBody: {
    randaoReveal: 'H256',
    eth1Data: 'H256',
    graffiti: 'BytesFixed1',
    proposerSlashings: 'H256',
    attesterSlashings: 'H256',
    attestations: 'H256',
    deposits: 'H256',
    voluntaryExits: 'H256',
    syncAggregate: 'H256',
    executionPayload: 'ExecutionPayload',
    blsToExecutionChanges: 'H256',
    blobKzgCommitments: 'H256',
    executionRequests: 'H256',
  },
  BytesFixed1: '(FixedArray1ForU8)',
  FixedArray1ForU8: '([u8; 32])',
  ExecutionPayload: {
    parentHash: 'BytesFixed1',
    feeRecipient: 'BytesFixed2',
    stateRoot: 'BytesFixed1',
    receiptsRoot: 'BytesFixed1',
    logsBloom: 'H256',
    prevRandao: 'BytesFixed1',
    blockNumber: 'u64',
    gasLimit: 'u64',
    gasUsed: 'u64',
    timestamp: 'u64',
    extraData: 'ByteList',
    baseFeePerGas: 'U256',
    blockHash: 'BytesFixed1',
    transactions: 'H256',
    withdrawals: 'H256',
    blobGasUsed: 'u64',
    excessBlobGas: 'u64',
  },
  BytesFixed2: '(FixedArray2ForU8)',
  FixedArray2ForU8: '([u8; 20])',
  ByteList: '(ListForU8)',
  ListForU8: { data: 'Vec<u8>' },
};

type Hash = Uint8Array;
export type U64 = bigint | number | string;
export type U256 = bigint;
type BytesFixed1 = Uint8Array;
type BytesFixed2 = Uint8Array;
type ByteList = Uint8Array;

export interface ExecutionPayload {
  parentHash: BytesFixed1;
  feeRecipient: BytesFixed2;
  stateRoot: BytesFixed1;
  receiptsRoot: BytesFixed1;
  logsBloom: Hash;
  prevRandao: BytesFixed1;
  blockNumber: U64;
  gasLimit: U64;
  gasUsed: U64;
  timestamp: U64;
  extraData: ByteList;
  baseFeePerGas: U256;
  blockHash: BytesFixed1;
  transactions: Hash;
  withdrawals: Hash;
  blobGasUsed: U64;
  excessBlobGas: U64;
}

export interface BlockBody {
  randaoReveal: Hash;
  eth1Data: Hash;
  graffiti: Uint8Array;
  proposerSlashings: Hash;
  attesterSlashings: Hash;
  attestations: Hash;
  deposits: Hash;
  voluntaryExits: Hash;
  syncAggregate: Hash;
  executionPayload: ExecutionPayload;
  blsToExecutionChanges: Hash;
  blobKzgCommitments: Hash;
  executionRequests: Hash;
}

export interface BlockGenericForBlockBody {
  slot: U64;
  proposerIndex: U64;
  parentRoot: Hash;
  stateRoot: Hash;
  body: BlockBody;
}

export interface BlockHeader {
  slot: U64;
  proposerIndex: U64;
  parentRoot: Hash;
  stateRoot: Hash;
  bodyRoot: Hash;
}

export interface BlockInclusionProof {
  block: BlockGenericForBlockBody;
  headers: BlockHeader[];
}

export interface ProofResult {
  proofBlock: BlockInclusionProof;
  proof: Uint8Array[];
  transactionIndex: U64;
  receiptRlp: Uint8Array;
}
