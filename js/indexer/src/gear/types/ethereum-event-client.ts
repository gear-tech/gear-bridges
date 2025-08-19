export interface Relayed {
  readonly slot: string;
  readonly block_number: number;
  readonly transaction_index: number;
  readonly fungible_token: string;
  readonly to: string;
  readonly amount: string;
}
