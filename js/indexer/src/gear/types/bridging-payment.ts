export interface BridgingPaidEvent {
  readonly nonce: string;
}

export interface PriorityBridgingPaid {
  readonly block: string;
  readonly nonce: string;
}
