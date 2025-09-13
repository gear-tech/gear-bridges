import { Extrinsic } from '@polkadot/types/interfaces';
import { useQuery } from '@tanstack/react-query';

import { definedAssert, isUndefined } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';

type Transaction = {
  extrinsic: Extrinsic | undefined;
  gasLimit: bigint;
  estimatedFee: bigint;
  value?: bigint;
};

const GAS_LIMIT = {
  BRIDGE: 150_000_000_000n,
  APPROXIMATE_PAY_FEE: 10_000_000_000n,
} as const;

type Params = {
  formValues: FormattedValues;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  vftManagerFee: bigint | undefined;
  allowance: bigint | undefined;
};

function useVaraTxs({ formValues, bridgingFee, shouldPayBridgingFee, vftManagerFee, allowance }: Params) {
  const { token } = useBridgeContext();

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();

  const getTxs = async () => {
    definedAssert(allowance, 'Allowance');
    definedAssert(bridgingFee, 'Bridging fee value');
    definedAssert(vftManagerFee, 'VFT Manager fee value');
    definedAssert(token, 'Fungible token');

    const { amount, accountAddress } = formValues;
    const txs: Transaction[] = [];
    const shouldMint = token.isNative;
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
        args: [CONTRACT_ADDRESS.VFT_MANAGER, amount],
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

    return txs;
  };

  return useQuery({
    queryKey: [
      'vara-txs',
      bridgingFee?.toString(),
      shouldPayBridgingFee,
      vftManagerFee?.toString(),
      allowance?.toString(),
      formValues.amount.toString(),
      formValues.accountAddress,
      token,
    ],

    queryFn: getTxs,
    enabled: Boolean(bridgingFee && vftManagerFee && !isUndefined(allowance) && token),
  });
}

export { useVaraTxs };
