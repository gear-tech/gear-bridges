const ERC20_MANAGER_ABI = [
  {
    type: 'constructor',
    inputs: [
      { name: 'message_queue', type: 'address', internalType: 'address' },
      { name: 'vft_manager', type: 'bytes32', internalType: 'bytes32' },
    ],
    stateMutability: 'nonpayable',
  },
  {
    type: 'function',
    name: 'getTokenSupplyType',
    inputs: [{ name: 'token', type: 'address', internalType: 'address' }],
    outputs: [{ name: '', type: 'uint8', internalType: 'enum IERC20Manager.SupplyType' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'processVaraMessage',
    inputs: [
      { name: 'sender', type: 'bytes32', internalType: 'bytes32' },
      { name: 'payload', type: 'bytes', internalType: 'bytes' },
    ],
    outputs: [{ name: '', type: 'bool', internalType: 'bool' }],
    stateMutability: 'nonpayable',
  },
  {
    type: 'function',
    name: 'requestBridging',
    inputs: [
      { name: 'token', type: 'address', internalType: 'address' },
      { name: 'amount', type: 'uint256', internalType: 'uint256' },
      { name: 'to', type: 'bytes32', internalType: 'bytes32' },
    ],
    outputs: [],
    stateMutability: 'nonpayable',
  },
  {
    type: 'function',
    name: 'requestBridgingPayingFee',
    inputs: [
      { name: 'token', type: 'address', internalType: 'address' },
      { name: 'amount', type: 'uint256', internalType: 'uint256' },
      { name: 'to', type: 'bytes32', internalType: 'bytes32' },
      { name: 'bridgingPayment', type: 'address', internalType: 'address' },
    ],
    outputs: [],
    stateMutability: 'payable',
  },
  {
    type: 'event',
    name: 'BridgingAccepted',
    inputs: [
      { name: 'to', type: 'address', indexed: true, internalType: 'address' },
      { name: 'token', type: 'address', indexed: true, internalType: 'address' },
      { name: 'amount', type: 'uint256', indexed: false, internalType: 'uint256' },
    ],
    anonymous: false,
  },
  {
    type: 'event',
    name: 'BridgingRequested',
    inputs: [
      { name: 'from', type: 'address', indexed: true, internalType: 'address' },
      { name: 'to', type: 'bytes32', indexed: true, internalType: 'bytes32' },
      { name: 'token', type: 'address', indexed: true, internalType: 'address' },
      { name: 'amount', type: 'uint256', indexed: false, internalType: 'uint256' },
    ],
    anonymous: false,
  },
  { type: 'error', name: 'BadArguments', inputs: [] },
  { type: 'error', name: 'BadVftManagerAddress', inputs: [] },
  { type: 'error', name: 'NotAuthorized', inputs: [] },
  {
    type: 'error',
    name: 'SafeERC20FailedOperation',
    inputs: [{ name: 'token', type: 'address', internalType: 'address' }],
  },
  { type: 'error', name: 'UnsupportedTokenSupply', inputs: [] },
] as const;

export { ERC20_MANAGER_ABI };
