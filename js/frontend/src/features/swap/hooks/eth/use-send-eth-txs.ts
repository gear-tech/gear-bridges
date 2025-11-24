import { useMutation } from '@tanstack/react-query';

import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { FormattedValues } from '../../types';

import { usePrepareEthTxs } from './use-prepare-eth-txs';

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  ftBalance: bigint | undefined;
  onTransactionStart: (values: FormattedValues) => void;
};

function useSendEthTxs({ bridgingFee, shouldPayBridgingFee, ftBalance, onTransactionStart }: Params) {
  const { syncEthWalletNetwork } = useNetworkType();

  const ethTsx = usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee, ftBalance });

  const sendTxs = async (values: FormattedValues) => {
    definedAssert(ethTsx.prepare, 'Prepared transactions');

    await syncEthWalletNetwork();

    const txs = await ethTsx.prepare(values);

    ethTsx.resetState();
    onTransactionStart(values);

    for (const { call } of txs) await call();
  };

  return { ...useMutation({ mutationFn: sendTxs }), status: ethTsx.status };
}

export { useSendEthTxs };
