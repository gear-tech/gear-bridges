export interface BridgingRequested {
  readonly nonce: string;
  readonly vara_token_id: string;
  readonly amount: string;
  readonly sender: string;
  readonly receiver: string;
}

export interface TokenMapping {
  readonly vara_token_id: string;
  readonly eth_token_id: string;
}
