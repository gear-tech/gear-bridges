export interface BridgingRequested {
  readonly nonce: string;
  readonly vara_token_id: string;
  readonly amount: string;
  readonly sender: string;
  readonly receiver: string;
}

export interface TokenMappingAdded {
  readonly vara_token_id: string;
  readonly eth_token_id: string;
  readonly supply_type: 'Ethereum' | 'Gear';
}

export type TokenMappingRemoved = TokenMappingAdded;

export interface HistoricalProxyAddressChanged {
  readonly new: string;
  readonly old: string;
}

export interface RequestBridgingArgs {
  readonly vara_token_id: string;
  readonly amount: string;
  readonly receiver: string;
}
