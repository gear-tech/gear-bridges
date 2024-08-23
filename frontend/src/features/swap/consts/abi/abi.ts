const ABI = [
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '_bridgeId',
        type: 'uint256',
      },
      {
        internalType: 'address',
        name: '_addressOfToken',
        type: 'address',
      },
      {
        internalType: 'uint8',
        name: '_signaturesThreshold',
        type: 'uint8',
      },
      {
        internalType: 'uint256',
        name: '_minAmount',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: '_fee',
        type: 'uint256',
      },
      {
        components: [
          {
            internalType: 'uint256',
            name: 'x',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'y',
            type: 'uint256',
          },
        ],
        internalType: 'struct LibSecp256k1.Point[]',
        name: '_configAuthorityKeys',
        type: 'tuple[]',
      },
      {
        internalType: 'address[]',
        name: '_emergencyAdmins',
        type: 'address[]',
      },
      {
        internalType: 'address[]',
        name: '_validatorPublicKeys',
        type: 'address[]',
      },
      {
        internalType: 'uint8',
        name: '_minValidatorsRequired',
        type: 'uint8',
      },
      {
        internalType: 'bool',
        name: '_requireMigration',
        type: 'bool',
      },
    ],
    stateMutability: 'nonpayable',
    type: 'constructor',
  },
  {
    inputs: [],
    name: 'AccessControlBadConfirmation',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'account',
        type: 'address',
      },
      {
        internalType: 'bytes32',
        name: 'neededRole',
        type: 'bytes32',
      },
    ],
    name: 'AccessControlUnauthorizedAccount',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'feeBalance',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
    ],
    name: 'AmountExceedsAvailableFeeBalance',
    type: 'error',
  },
  {
    inputs: [],
    name: 'AuthorityKeyAlreadyExists',
    type: 'error',
  },
  {
    inputs: [],
    name: 'CallerNotValidator',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ContractAlreadyInEmergencyMode',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ContractInMigration',
    type: 'error',
  },
  {
    inputs: [],
    name: 'DuplicatePublicKeyIDs',
    type: 'error',
  },
  {
    inputs: [],
    name: 'DuplicateSignaturesDetected',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ECDSAInvalidSignature',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'length',
        type: 'uint256',
      },
    ],
    name: 'ECDSAInvalidSignatureLength',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 's',
        type: 'bytes32',
      },
    ],
    name: 'ECDSAInvalidSignatureS',
    type: 'error',
  },
  {
    inputs: [],
    name: 'EmergencyStopActive',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ErrorDuringMigration',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'to',
        type: 'address',
      },
    ],
    name: 'EtherSendFailure',
    type: 'error',
  },
  {
    inputs: [],
    name: 'IndexOutOfBounds',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'provided',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'required',
        type: 'uint256',
      },
    ],
    name: 'InsufficientFee',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'required',
        type: 'uint256',
      },
    ],
    name: 'InsufficientSigners',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint8',
        name: 'threshold',
        type: 'uint8',
      },
      {
        internalType: 'uint256',
        name: 'signersAmount',
        type: 'uint256',
      },
    ],
    name: 'InsufficientSignersError',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'from',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'to',
        type: 'uint256',
      },
    ],
    name: 'InvalidIndexRange',
    type: 'error',
  },
  {
    inputs: [],
    name: 'MigrationNotActive',
    type: 'error',
  },
  {
    inputs: [],
    name: 'MigrationOfNullElements',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    name: 'NonAuthorisedValidator',
    type: 'error',
  },
  {
    inputs: [],
    name: 'NonSequentialNonceArray',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'caller',
        type: 'address',
      },
    ],
    name: 'NotAuthority',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    name: 'NotEmerencyAdmin',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'caller',
        type: 'address',
      },
    ],
    name: 'NotEmergencyAdmin',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'balance',
        type: 'uint256',
      },
    ],
    name: 'NotEnoughBalance',
    type: 'error',
  },
  {
    inputs: [],
    name: 'NotInEmergencyStop',
    type: 'error',
  },
  {
    inputs: [],
    name: 'PointNotOnCurve',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ReentrancyGuardReentrantCall',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'expected',
        type: 'address',
      },
      {
        internalType: 'address',
        name: 'provided',
        type: 'address',
      },
    ],
    name: 'SignatureMismatch',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    name: 'SignersReducedBelowThreshold',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    name: 'ThresholdExceedsSignerCount',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'minAmount',
        type: 'uint256',
      },
    ],
    name: 'TooLowAmount',
    type: 'error',
  },
  {
    inputs: [],
    name: 'TransfersArrayEmpty',
    type: 'error',
  },
  {
    inputs: [],
    name: 'TryingToMigrateNullElement',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'expectedPreviousContract',
        type: 'address',
      },
      {
        internalType: 'address',
        name: 'attemptedFrom',
        type: 'address',
      },
    ],
    name: 'UnauthorizedMigration',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    name: 'ValidatorExists',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'validatorsRequired',
        type: 'uint256',
      },
    ],
    name: 'ValidatorsNotReached',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'string',
        name: '',
        type: 'string',
      },
    ],
    name: 'VaraAddressNotValid',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'string',
        name: '',
        type: 'string',
      },
    ],
    name: 'VerificationFailed',
    type: 'error',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'expected',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'provided',
        type: 'uint256',
      },
    ],
    name: 'WrongMinNonceId',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ZeroAmount',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ZeroSignatureThreshold',
    type: 'error',
  },
  {
    inputs: [],
    name: 'ZeroThreshold',
    type: 'error',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: false,
        internalType: 'address',
        name: 'sender',
        type: 'address',
      },
      {
        indexed: false,
        internalType: 'string',
        name: 'recipient',
        type: 'string',
      },
      {
        indexed: false,
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
      {
        indexed: false,
        internalType: 'uint256',
        name: 'nonceId',
        type: 'uint256',
      },
    ],
    name: 'EthToVaraTransferEvent',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: true,
        internalType: 'address',
        name: 'to',
        type: 'address',
      },
      {
        indexed: false,
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
    ],
    name: 'EtherWithdrawn',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: false,
        internalType: 'uint256',
        name: 'newFee',
        type: 'uint256',
      },
    ],
    name: 'FeeUpdated',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: true,
        internalType: 'address',
        name: 'sender',
        type: 'address',
      },
      {
        indexed: false,
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
    ],
    name: 'Received',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: true,
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        indexed: true,
        internalType: 'bytes32',
        name: 'previousAdminRole',
        type: 'bytes32',
      },
      {
        indexed: true,
        internalType: 'bytes32',
        name: 'newAdminRole',
        type: 'bytes32',
      },
    ],
    name: 'RoleAdminChanged',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: true,
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        indexed: true,
        internalType: 'address',
        name: 'account',
        type: 'address',
      },
      {
        indexed: true,
        internalType: 'address',
        name: 'sender',
        type: 'address',
      },
    ],
    name: 'RoleGranted',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        indexed: true,
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        indexed: true,
        internalType: 'address',
        name: 'account',
        type: 'address',
      },
      {
        indexed: true,
        internalType: 'address',
        name: 'sender',
        type: 'address',
      },
    ],
    name: 'RoleRevoked',
    type: 'event',
  },
  {
    anonymous: false,
    inputs: [
      {
        components: [
          {
            internalType: 'string',
            name: 'sender',
            type: 'string',
          },
          {
            internalType: 'address payable',
            name: 'recipient',
            type: 'address',
          },
          {
            internalType: 'uint256',
            name: 'amount',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'nonceId',
            type: 'uint256',
          },
        ],
        indexed: false,
        internalType: 'struct Bridge.VaraToEthTransfer[]',
        name: 'varaToEthTransfers',
        type: 'tuple[]',
      },
    ],
    name: 'VaraToEthTransferEvent',
    type: 'event',
  },
  {
    inputs: [],
    name: 'DEFAULT_ADMIN_ROLE',
    outputs: [
      {
        internalType: 'bytes32',
        name: '',
        type: 'bytes32',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'EMERGENCY_ADMIN_ROLE',
    outputs: [
      {
        internalType: 'bytes32',
        name: '',
        type: 'bytes32',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
    ],
    name: 'activateEmergencyStopByConfigAuthority',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [],
    name: 'activeEmergencyStopByEmergencyAdmin',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        components: [
          {
            internalType: 'uint256',
            name: 'x',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'y',
            type: 'uint256',
          },
        ],
        internalType: 'struct LibSecp256k1.Point',
        name: '_configAuthorityKey',
        type: 'tuple',
      },
    ],
    name: 'addConfigAuthorityKey',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'address',
        name: 'newValidatorAddress',
        type: 'address',
      },
    ],
    name: 'addValidator',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [],
    name: 'addressOfToken',
    outputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'authorityNonce',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'bridgeId',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes[]',
        name: 'signatures',
        type: 'bytes[]',
      },
    ],
    name: 'checkUniqueness',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    name: 'configAuthorityKeys',
    outputs: [
      {
        internalType: 'uint256',
        name: 'x',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'y',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
    ],
    name: 'deactivateEmergencyStop',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [],
    name: 'emergencyStopped',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
    ],
    name: 'endMigration',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [],
    name: 'ethToVaraNonce',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    name: 'ethToVaraTransfers',
    outputs: [
      {
        internalType: 'address',
        name: 'sender',
        type: 'address',
      },
      {
        internalType: 'string',
        name: 'recipient',
        type: 'string',
      },
      {
        internalType: 'uint256',
        name: 'amount',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'nonceId',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'fee',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'feeBalance',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'getAllValidatorPublicKeys',
    outputs: [
      {
        internalType: 'address[]',
        name: '',
        type: 'address[]',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'getAuthorityEthAddresses',
    outputs: [
      {
        internalType: 'address[]',
        name: '',
        type: 'address[]',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'getConfigAuthorityKeys',
    outputs: [
      {
        components: [
          {
            internalType: 'uint256',
            name: 'x',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'y',
            type: 'uint256',
          },
        ],
        internalType: 'struct LibSecp256k1.Point[]',
        name: '',
        type: 'tuple[]',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'fromIndex',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'toIndex',
        type: 'uint256',
      },
    ],
    name: 'getEthToVaraTransfersInRange',
    outputs: [
      {
        components: [
          {
            internalType: 'address',
            name: 'sender',
            type: 'address',
          },
          {
            internalType: 'string',
            name: 'recipient',
            type: 'string',
          },
          {
            internalType: 'uint256',
            name: 'amount',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'nonceId',
            type: 'uint256',
          },
        ],
        internalType: 'struct Bridge.EthToVaraTransfer[]',
        name: '',
        type: 'tuple[]',
      },
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'getLastEthToVaraNonce',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'getLastVaraToEthNonce',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
    ],
    name: 'getRoleAdmin',
    outputs: [
      {
        internalType: 'bytes32',
        name: '',
        type: 'bytes32',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'address',
        name: 'newEmergencyAdmin',
        type: 'address',
      },
    ],
    name: 'grantEmergencyAdminRole',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        internalType: 'address',
        name: 'account',
        type: 'address',
      },
    ],
    name: 'grantRole',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        internalType: 'address',
        name: 'account',
        type: 'address',
      },
    ],
    name: 'hasRole',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'inMigration',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        components: [
          {
            internalType: 'uint256',
            name: 'x',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'y',
            type: 'uint256',
          },
        ],
        internalType: 'struct LibSecp256k1.Point',
        name: '_configAuthorityKey',
        type: 'tuple',
      },
    ],
    name: 'isKeyNotPresent',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'string',
        name: '_addr',
        type: 'string',
      },
    ],
    name: 'isValidVaraAddress',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'pure',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: '_validator',
        type: 'address',
      },
    ],
    name: 'isValidator',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'authorityKeyId',
        type: 'uint256',
      },
      {
        internalType: 'address',
        name: 'newContract',
        type: 'address',
      },
      {
        internalType: 'uint256',
        name: 'start',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: 'count',
        type: 'uint256',
      },
    ],
    name: 'migrateEthToVaraTransfers',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: 'authorityKeyId',
        type: 'uint256',
      },
      {
        internalType: 'address',
        name: 'newContract',
        type: 'address',
      },
    ],
    name: 'migrateNonces',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [],
    name: 'minAmount',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'minValidatorsRequired',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'previousContract',
    outputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        components: [
          {
            internalType: 'address',
            name: 'sender',
            type: 'address',
          },
          {
            internalType: 'string',
            name: 'recipient',
            type: 'string',
          },
          {
            internalType: 'uint256',
            name: 'amount',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'nonceId',
            type: 'uint256',
          },
        ],
        internalType: 'struct Bridge.EthToVaraTransfer[]',
        name: 'transfers',
        type: 'tuple[]',
      },
    ],
    name: 'receiveEthToVaraTransfers',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'uint256',
        name: '_configAuthorityKeyId',
        type: 'uint256',
      },
    ],
    name: 'removeConfigAuthorityKey',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'address',
        name: 'validatorAddress',
        type: 'address',
      },
    ],
    name: 'removeValidator',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        internalType: 'address',
        name: 'callerConfirmation',
        type: 'address',
      },
    ],
    name: 'renounceRole',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'address',
        name: 'emergencyAdmin',
        type: 'address',
      },
    ],
    name: 'revokeEmergencyAdminRole',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'role',
        type: 'bytes32',
      },
      {
        internalType: 'address',
        name: 'account',
        type: 'address',
      },
    ],
    name: 'revokeRole',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '_ethToVaraNonce',
        type: 'uint256',
      },
      {
        internalType: 'uint256',
        name: '_varaToEthNonce',
        type: 'uint256',
      },
    ],
    name: 'setNonces',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'address',
        name: '_previousContract',
        type: 'address',
      },
    ],
    name: 'setPreviousContract',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [],
    name: 'signaturesThreshold',
    outputs: [
      {
        internalType: 'uint8',
        name: '',
        type: 'uint8',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        components: [
          {
            internalType: 'address',
            name: 'sender',
            type: 'address',
          },
          {
            internalType: 'string',
            name: 'recipient',
            type: 'string',
          },
          {
            internalType: 'uint256',
            name: 'amount',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'nonceId',
            type: 'uint256',
          },
        ],
        internalType: 'struct Bridge.EthToVaraTransfer[]',
        name: 'transfers',
        type: 'tuple[]',
      },
    ],
    name: 'sortTransactionNonces',
    outputs: [
      {
        internalType: 'uint256[]',
        name: '',
        type: 'uint256[]',
      },
    ],
    stateMutability: 'pure',
    type: 'function',
  },
  {
    inputs: [
      {
        components: [
          {
            internalType: 'string',
            name: 'sender',
            type: 'string',
          },
          {
            internalType: 'address payable',
            name: 'recipient',
            type: 'address',
          },
          {
            internalType: 'uint256',
            name: 'amount',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'nonceId',
            type: 'uint256',
          },
        ],
        internalType: 'struct Bridge.VaraToEthTransfer[]',
        name: 'transfers',
        type: 'tuple[]',
      },
    ],
    name: 'sortTransactionNonces',
    outputs: [
      {
        internalType: 'uint256[]',
        name: '',
        type: 'uint256[]',
      },
    ],
    stateMutability: 'pure',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
    ],
    name: 'startMigration',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes4',
        name: 'interfaceId',
        type: 'bytes4',
      },
    ],
    name: 'supportsInterface',
    outputs: [
      {
        internalType: 'bool',
        name: '',
        type: 'bool',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'string',
        name: '_recipient',
        type: 'string',
      },
      {
        internalType: 'uint256',
        name: '_amount',
        type: 'uint256',
      },
    ],
    name: 'transitEthToVara',
    outputs: [],
    stateMutability: 'payable',
    type: 'function',
  },
  {
    inputs: [
      {
        components: [
          {
            internalType: 'string',
            name: 'sender',
            type: 'string',
          },
          {
            internalType: 'address payable',
            name: 'recipient',
            type: 'address',
          },
          {
            internalType: 'uint256',
            name: 'amount',
            type: 'uint256',
          },
          {
            internalType: 'uint256',
            name: 'nonceId',
            type: 'uint256',
          },
        ],
        internalType: 'struct Bridge.VaraToEthTransfer[]',
        name: '_varaToEthTransfers',
        type: 'tuple[]',
      },
      {
        internalType: 'bytes[]',
        name: 'signatures',
        type: 'bytes[]',
      },
      {
        internalType: 'uint256',
        name: 'lastExecutedVaraNonceId',
        type: 'uint256',
      },
    ],
    name: 'transitVaraToEthBatch',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'uint256',
        name: 'newFee',
        type: 'uint256',
      },
    ],
    name: 'updateFee',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'uint256',
        name: '_minAmount',
        type: 'uint256',
      },
    ],
    name: 'updateMinAmount',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'uint8',
        name: '_minValidatorsRequired',
        type: 'uint8',
      },
    ],
    name: 'updateMinValidatorsRequired',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        internalType: 'uint8',
        name: '_newThreshold',
        type: 'uint8',
      },
    ],
    name: 'updateSignaturesThreshold',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: 'newValidatorAddress',
        type: 'address',
      },
    ],
    name: 'updateValidatorKey',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
      {
        components: [
          {
            internalType: 'address',
            name: 'previousValidatorAddress',
            type: 'address',
          },
          {
            internalType: 'address',
            name: 'newValidatorAddress',
            type: 'address',
          },
        ],
        internalType: 'struct Bridge.ValidatorKeyUpdateByAutority[]',
        name: 'validatorKeyUpdate',
        type: 'tuple[]',
      },
    ],
    name: 'updateValidatorKeysByAuthority',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    name: 'validatorIndices',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    name: 'validatorPublicKeysArray',
    outputs: [
      {
        internalType: 'address',
        name: '',
        type: 'address',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [],
    name: 'varaToEthNonce',
    outputs: [
      {
        internalType: 'uint256',
        name: '',
        type: 'uint256',
      },
    ],
    stateMutability: 'view',
    type: 'function',
  },
  {
    inputs: [
      {
        internalType: 'address payable',
        name: '_to',
        type: 'address',
      },
      {
        internalType: 'uint256',
        name: '_amount',
        type: 'uint256',
      },
      {
        internalType: 'bytes32',
        name: 'signature',
        type: 'bytes32',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentAggregated',
        type: 'bytes20',
      },
      {
        internalType: 'bytes20',
        name: 'commitmentSigners',
        type: 'bytes20',
      },
      {
        internalType: 'uint256[]',
        name: 'pubKeysIds',
        type: 'uint256[]',
      },
    ],
    name: 'withdrawEther',
    outputs: [],
    stateMutability: 'nonpayable',
    type: 'function',
  },
] as const;

export { ABI };
