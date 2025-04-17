const BRIDGING_PAYMENT_ABI = [
  {
    type: 'function',
    name: 'erc20Manager',
    inputs: [],
    outputs: [{ name: '', type: 'address', internalType: 'address' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'fee',
    inputs: [],
    outputs: [{ name: '', type: 'uint256', internalType: 'uint256' }],
    stateMutability: 'view',
  },
  { type: 'function', name: 'payFee', inputs: [], outputs: [], stateMutability: 'payable' },
  {
    type: 'function',
    name: 'setFee',
    inputs: [{ name: 'newFee', type: 'uint256', internalType: 'uint256' }],
    outputs: [],
    stateMutability: 'nonpayable',
  },
  { type: 'event', name: 'FeePaid', inputs: [], anonymous: false },
] as const;

export { BRIDGING_PAYMENT_ABI };
