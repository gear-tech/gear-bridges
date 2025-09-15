import { HexString } from '@gear-js/api';
import { Extrinsic } from '@polkadot/types/interfaces';

import { isUndefined } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';

const GAS_LIMIT = {
  BRIDGE: 150_000_000_000n,
  APPROXIMATE_PAY_FEE: 10_000_000_000n,
} as const;

type Transaction = {
  extrinsic: Extrinsic | undefined;
  gasLimit: bigint;
  estimatedFee: bigint;
  value?: bigint;
};

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  vftManagerFee: bigint | undefined;
  allowance: bigint | undefined;
};

function usePrepareVaraTxs({ bridgingFee, shouldPayBridgingFee, vftManagerFee, allowance }: Params) {
  const { token } = useBridgeContext();

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();

  if (isUndefined(bridgingFee) || isUndefined(vftManagerFee) || isUndefined(allowance) || !token) return;

  return async ({ amount, accountAddress, accountOverride }: FormattedValues & { accountOverride?: HexString }) => {
    const accountArg = accountOverride ? { account: { addressOrPair: accountOverride } } : {};

    const txs: Transaction[] = [];
    const shouldMint = token.isNative;
    const shouldApprove = amount > allowance;

    if (shouldMint) {
      const { transaction, fee } = await mint.prepareTransactionAsync({ ...accountArg, args: [], value: amount });

      txs.push({
        extrinsic: transaction.extrinsic,
        gasLimit: transaction.gasInfo.min_limit.toBigInt(),
        estimatedFee: fee,
        value: amount,
      });
    }

    if (shouldApprove) {
      const { transaction, fee } = await approve.prepareTransactionAsync({
        ...accountArg,
        args: [CONTRACT_ADDRESS.VFT_MANAGER, amount],
      });

      txs.push({
        extrinsic: transaction.extrinsic,
        gasLimit: transaction.gasInfo.min_limit.toBigInt(),
        estimatedFee: fee,
      });
    }

    const { transaction, fee } = await requestBridging.prepareTransactionAsync({
      ...accountArg,
      gasLimit: GAS_LIMIT.BRIDGE,
      args: [token.address, amount, accountAddress],
      value: vftManagerFee,
    });

    txs.push({
      extrinsic: transaction.extrinsic,
      gasLimit: GAS_LIMIT.BRIDGE,
      estimatedFee: fee,
      value: vftManagerFee,
    });

    // using approximate values, cuz we don't know the exact gas limit yet
    const feesTx = {
      extrinsic: undefined,
      gasLimit: GAS_LIMIT.APPROXIMATE_PAY_FEE,
      estimatedFee: fee,
    };

    if (shouldPayBridgingFee) txs.push({ ...feesTx, value: bridgingFee });

    return txs;
  };
}

export { usePrepareVaraTxs };
