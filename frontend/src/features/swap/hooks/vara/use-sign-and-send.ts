import { useAccount, useApi } from '@gear-js/react-hooks';
import { AddressOrPair, SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

type Parameters = {
  extrinsic: Extrinsic;
  addressOrPair?: AddressOrPair;
};

const DEFAULT_OPTIONS = {
  addressOrPair: undefined,
} as const;

function useSignAndSend() {
  const { api } = useApi();
  const { account } = useAccount();

  const signAndSend = ({ extrinsic, ...parameters }: Parameters) =>
    new Promise<void>((resolve, reject) => {
      if (!api) throw new Error('API is not initialized');
      if (!account) throw new Error('Account is not found');

      const { address, signer } = account;
      const { addressOrPair } = { ...DEFAULT_OPTIONS, ...parameters };

      const statusCallback = (result: ISubmittableResult) => {
        const { events, status } = result;
        if (!status.isInBlock) return;

        events.forEach(({ event }) => {
          if (event.method === 'ExtrinsicFailed') {
            const { method, docs } = api.getExtrinsicFailedError(event);
            const errorMessage = `${method}: ${docs}`;

            reject(new Error(errorMessage));
          }

          if (event.method === 'ExtrinsicSuccess') resolve();
        });
      };

      const _signAndSend = () =>
        addressOrPair
          ? extrinsic.signAndSend(addressOrPair, statusCallback)
          : extrinsic.signAndSend(address, { signer }, statusCallback);

      _signAndSend().catch((error) => reject(error instanceof Error ? error : new Error(String(error))));
    });

  return useMutation({
    mutationKey: ['signAndSend'],
    mutationFn: signAndSend,
  });
}

export { useSignAndSend };
