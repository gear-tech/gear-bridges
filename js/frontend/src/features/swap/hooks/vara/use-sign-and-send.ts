import { HexString, MessageQueuedData } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { AddressOrPair, SubmittableExtrinsic } from '@polkadot/api/types';
import { TypeRegistry } from '@polkadot/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';
import { throwOnErrorReply } from 'sails-js';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

type Parameters = {
  programs: ({ programId: HexString; registry: TypeRegistry } | undefined)[];
};

type SignAndSendParameters = {
  extrinsic: Extrinsic;
  addressOrPair?: AddressOrPair;
};

const DEFAULT_OPTIONS = {
  addressOrPair: undefined,
} as const;

function useSignAndSend({ programs }: Parameters) {
  const { api } = useApi();
  const { account } = useAccount();

  // to handle errors in case of not enough gas
  const checkErrorReplies = (blockHash: HexString, queuedMessages: MessageQueuedData[]) => {
    if (!api) throw new Error('API is not initialized');

    const promises = queuedMessages.map(async ({ destination, id }) => {
      const program = programs.find((_program) => _program!.programId === destination.toHex());

      if (!program) return;
      const { programId, registry } = program;

      const reply = await api.message.getReplyEvent(programId, id.toHex(), blockHash);
      const { details, payload } = reply.data.message;

      return throwOnErrorReply(details.unwrap().code, payload, api.specVersion, registry);
    });

    return Promise.all(promises);
  };

  const signAndSend = ({ extrinsic, ...parameters }: SignAndSendParameters) =>
    new Promise<{ blockHash: HexString }>((resolve, reject) => {
      if (!api) throw new Error('API is not initialized');
      if (!account) throw new Error('Account is not found');
      if (programs.includes(undefined)) throw new Error('Each program is not found');

      const { address, signer } = account;
      const { addressOrPair } = { ...DEFAULT_OPTIONS, ...parameters };

      const statusCallback = (result: ISubmittableResult) => {
        const { events, status } = result;
        if (!status.isInBlock) return;

        const queuedMessages: MessageQueuedData[] = [];

        events.forEach(({ event }) => {
          const { method, section } = event;

          if (method === 'MessageQueued' && section === 'gear') queuedMessages.push(event.data as MessageQueuedData);

          if (method === 'ExtrinsicFailed') {
            const error = api.getExtrinsicFailedError(event);
            const message = `${error.method}: ${error.docs}`;

            reject(new Error(message));
          }

          if (method === 'ExtrinsicSuccess') {
            const blockHash = status.asInBlock.toHex();

            checkErrorReplies(blockHash, queuedMessages)
              .then(() => resolve({ blockHash }))
              .catch((error: Error) => reject(error));
          }
        });
      };

      const _signAndSend = () =>
        addressOrPair
          ? extrinsic.signAndSend(addressOrPair, statusCallback)
          : extrinsic.signAndSend(address, { signer }, statusCallback);

      _signAndSend().catch((error: Error) => reject(error));
    });

  return useMutation({
    mutationKey: ['signAndSend'],
    mutationFn: signAndSend,
  });
}

export { useSignAndSend };
