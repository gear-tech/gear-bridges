export interface BeaconGenesisBlock {
  readonly genesis_time: string;
  readonly genesis_validators_root: string;
  readonly genesis_fork_version: string;
}

export interface BeaconBlockHeader {
  readonly root: string;
  readonly canonical: boolean;
  readonly header: {
    readonly message: {
      readonly slot: string;
      readonly proposer_index: string;
      readonly parent_root: `0x${string}`;
      readonly state_root: `0x${string}`;
      readonly body_root: `0x${string}`;
    };
    readonly signature: string;
  };
}

export interface IBeaconBlockBody {
  readonly randao_reveal: string;
  readonly eth1_data: {
    readonly deposit_root: string;
    readonly deposit_count: string;
    readonly block_hash: string;
  };
  readonly graffiti: string;
  readonly proposer_slashings: unknown[];
  readonly attester_slashings: unknown[];
  readonly attestations: {
    readonly aggregation_bits: string;
    readonly data: {
      readonly slot: string;
      readonly index: string;
      readonly beacon_block_root: string;
      readonly source: {
        readonly epoch: string;
        readonly root: string;
      };
      readonly target: {
        readonly epoch: string;
        readonly root: string;
      };
    };
    readonly signature: string;
    readonly committee_bits: string;
  }[];
  readonly deposits: unknown[];
  readonly voluntary_exits: unknown[];
  readonly sync_aggregate: {
    readonly sync_committee_bits: string;
    readonly sync_committee_signature: string;
  };
  readonly execution_payload: {
    readonly parent_hash: string;
    readonly fee_recipient: string;
    readonly state_root: string;
    readonly receipts_root: string;
    readonly logs_bloom: string;
    readonly prev_randao: string;
    readonly block_number: string;
    readonly gas_limit: string;
    readonly gas_used: string;
    readonly timestamp: string;
    readonly extra_data: string;
    readonly base_fee_per_gas: string;
    readonly block_hash: string;
    readonly transactions: string[];
    readonly withdrawals: {
      readonly index: string;
      readonly validator_index: string;
      readonly address: string;
      readonly amount: string;
    }[];
    readonly blob_gas_used: string;
    readonly excess_blob_gas: string;
  };
  readonly bls_to_execution_changes: string;
  readonly blob_kzg_commitments: string;
  readonly execution_requests: {
    readonly deposits: unknown[];
    readonly withdrawals: unknown[];
    readonly consolidations: unknown[];
  };
}

export interface IBeaconBlock {
  readonly slot: string;
  readonly proposer_index: string;
  readonly parent_root: `0x${string}`;
  readonly state_root: `0x${string}`;
  readonly body: IBeaconBlockBody;
  readonly signatures: string[];
}

export interface MerkleRootLogArgs {
  blockNumber: bigint;
  merkleRoot: `0x${string}`;
}

export interface MessageProcessResult {
  success: boolean;
  transactionHash: `0x${string}`;
  blockNumber?: bigint;
  messageHash?: `0x${string}`;
  messageNonce?: bigint;
  messageDestination?: `0x${string}`;
  error?: string;
}
