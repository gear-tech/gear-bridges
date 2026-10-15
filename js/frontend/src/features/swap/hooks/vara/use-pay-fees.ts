import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { U256 } from '@polkadot/types';
import { H160, H256 } from '@polkadot/types/interfaces';

import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { Extrinsic } from '../../types';

import { usePreparePayPriorityFee } from './use-prepare-pay-priority-fee';
import { usePreparePayVaraFee } from './use-prepare-pay-vara-fee';
import { useSignAndSend } from './use-sign-and-send';

type Params = {
  fee: bigint | undefined;
  priorityFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  shouldPayPriorityFee: boolean;
};

function usePayFees({ fee, priorityFee, shouldPayBridgingFee, shouldPayPriorityFee }: Params) {
  const { api } = useApi();
  const { NETWORK_PRESET } = useNetworkType();

  const payFee = usePreparePayVaraFee();
  const payPriorityFees = usePreparePayPriorityFee();
  const signAndSend = useSignAndSend({ programs: [payFee.program, payPriorityFees.program] });

  const error = signAndSend.error || payFee.error || payPriorityFees.error;
  const isPending = signAndSend.isPending || payFee.isPending || payPriorityFees.isPending;

  const reset = () => {
    signAndSend.reset();
    payFee.reset();
    payPriorityFees.reset();
  };

  const payFees = async ({
    nonce,
    requestBlockHash,
  }: {
    nonce: bigint;
    requestBlockHash: HexString;
  }): Promise<void> => {
    if (!shouldPayBridgingFee && !shouldPayPriorityFee) return;
    definedAssert(api, 'API');
    const extrinsics: Extrinsic[] = [];
    const destinations: HexString[] = [];

    if (shouldPayBridgingFee) {
      definedAssert(fee, 'Fee');
      const { transaction } = await payFee.prepareTransactionAsync({ args: [nonce], value: fee });
      extrinsics.push(transaction.extrinsic);
      destinations.push(transaction.programId);
    }

    if (shouldPayPriorityFee) {
      definedAssert(priorityFee, 'Priority fee');
      const start = (await api.rpc.chain.getHeader(requestBlockHash)).number.toNumber();
      const head = (await api.rpc.chain.getHeader(await api.rpc.chain.getFinalizedHead())).number.toNumber();
      let queueBlockHash: HexString | undefined;
      for (let height = start; height <= head; height++) {
        const hash = (await api.rpc.chain.getBlockHash(height)).toHex();
        const messages = (await api.blocks.getEvents(hash))
          .filter(({ event }) => event.section === 'gearEthBridge' && event.method === 'MessageQueued')
          .map(({ event }) => event.data[0] as unknown as { nonce: U256; source: H256; destination: H160 })
          .filter((message) => message.nonce.toBigInt() === nonce);
        if (!messages.length) continue;
        const [message] = messages;
        if (
          messages.length !== 1 ||
          queueBlockHash ||
          message.source.toHex().toLowerCase() !== NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS.toLowerCase() ||
          message.destination.toHex().toLowerCase() !== NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS.toLowerCase()
        )
          throw new Error('Conflicting original queue insertion for priority payment');
        queueBlockHash = hash;
      }
      if (!queueBlockHash) throw new Error('Finalized original queue insertion is not available for priority payment');
      const { transaction } = await payPriorityFees.prepareTransactionAsync({
        args: [queueBlockHash, nonce],
        value: priorityFee,
        gasLimit: { increaseGas: 10 },
      });
      extrinsics.push(transaction.extrinsic);
      destinations.push(transaction.programId);
    }

    await signAndSend.mutateAsync({ extrinsic: api.tx.utility.batchAll(extrinsics), destinations });
  };

  return { payFees, error, isPending, reset };
}

export { usePayFees };
