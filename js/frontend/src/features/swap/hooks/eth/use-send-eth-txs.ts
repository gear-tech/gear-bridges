import { useMutation } from '@tanstack/react-query';

import { definedAssert } from '@/utils';

import { FormattedValues } from '../../types';

import { usePrepareEthTxs } from './use-prepare-eth-txs';

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  onTransactionStart: (values: FormattedValues) => void;
};

function useSendEthTxs({ bridgingFee, shouldPayBridgingFee, onTransactionStart }: Params) {
  const ethTsx = usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee });

  const sendTxs = async (values: FormattedValues) => {
    definedAssert(ethTsx.prepare, 'Prepared transactions');

    const txs = await ethTsx.prepare(values);

    ethTsx.resetState();
    onTransactionStart(values);

    for (const { call } of txs) await call();
  };

  return { ...useMutation({ mutationFn: sendTxs }), status: ethTsx.status };
}

export { useSendEthTxs };
