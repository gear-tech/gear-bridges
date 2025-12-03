import { Extrinsic } from '@polkadot/types/interfaces';

import { useNetworkType } from '@/context/network-type';
import { isUndefined } from '@/utils';

import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { useGetVaraFTAllowance } from './use-get-vara-ft-allowance';
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
  priorityFee: bigint | undefined;
  shouldPayPriorityFee: boolean;
  vftManagerFee: bigint | undefined;
};

function usePrepareVaraTxs({
  bridgingFee,
  shouldPayBridgingFee,
  priorityFee,
  shouldPayPriorityFee,
  vftManagerFee,
}: Params) {
  const { NETWORK_PRESET } = useNetworkType();
  const { token } = useBridgeContext();

  const getAllowance = useGetVaraFTAllowance(token?.address);

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();

  if (isUndefined(bridgingFee) || isUndefined(vftManagerFee) || isUndefined(priorityFee) || !token) return;

  return async ({ amount, accountAddress }: FormattedValues) => {
    const txs: Transaction[] = [];
    const shouldMint = token.isNative;

    const allowance = await getAllowance();
    const shouldApprove = amount > allowance;

    if (shouldMint) {
      const { transaction, fee } = await mint.prepareTransactionAsync({ args: [], value: amount });

      txs.push({
        extrinsic: transaction.extrinsic,
        gasLimit: transaction.gasInfo.min_limit.toBigInt(),
        estimatedFee: fee,
        value: amount,
      });
    }

    if (shouldApprove) {
      const { transaction, fee } = await approve.prepareTransactionAsync({
        args: [NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS, amount],
      });

      txs.push({
        extrinsic: transaction.extrinsic,
        gasLimit: transaction.gasInfo.min_limit.toBigInt(),
        estimatedFee: fee,
      });
    }

    const { transaction, fee } = await requestBridging.prepareTransactionAsync({
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
    if (shouldPayPriorityFee) txs.push({ ...feesTx, value: priorityFee });

    return txs;
  };
}

export { usePrepareVaraTxs };
