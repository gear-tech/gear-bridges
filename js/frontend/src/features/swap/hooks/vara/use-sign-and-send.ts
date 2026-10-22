import { HexString, MessageQueuedData } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { AddressOrPair, SubmittableExtrinsic } from '@polkadot/api/types';
import { TypeRegistry } from '@polkadot/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';
import { throwOnErrorReply } from 'sails-js';

import { signAndSendFinalized, waitForFinalizedReplies } from '../../submission';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

type Parameters = {
  programs: ({ programId: HexString; registry: TypeRegistry } | undefined)[];
};

type SignAndSendParameters = {
  extrinsic: Extrinsic;
  addressOrPair?: AddressOrPair;
  destinations: HexString[];
};

const DEFAULT_OPTIONS = {
  addressOrPair: undefined,
} as const;

function useSignAndSend({ programs }: Parameters) {
  const { api } = useApi();
  const { account } = useAccount();

  const signAndSend = async ({ extrinsic, destinations, ...parameters }: SignAndSendParameters) => {
    if (!api) throw new Error('API is not initialized');
    if (!account) throw new Error('Account is not found');
    const { address, signer } = account;
    const { addressOrPair } = { ...DEFAULT_OPTIONS, ...parameters };
    const deadline = Date.now() + 44 * 60 * 1000;
    const {
      events,
      status,
      txHash: originalHash,
    } = await signAndSendFinalized(extrinsic, addressOrPair ?? address, addressOrPair ? {} : { signer }, deadline);
    const blockHash = status.asFinalized.toHex();
    const txHash = originalHash.toHex();
    try {
      const failed = events.find(({ event }) => event.section === 'system' && event.method === 'ExtrinsicFailed');
      if (failed) {
        const error = api.getExtrinsicFailedError(failed.event);
        throw new Error(`${error.method}: ${error.docs}`);
      }
      if (!events.some(({ event }) => event.section === 'system' && event.method === 'ExtrinsicSuccess'))
        throw new Error('Missing finalized extrinsic success');
      const queued = events
        .filter(({ event }) => event.section === 'gear' && event.method === 'MessageQueued')
        .map(({ event }) => event.data as MessageQueuedData);
      if (
        queued.length !== destinations.length ||
        queued.some((message, i) => message.destination.toHex().toLowerCase() !== destinations[i].toLowerCase())
      )
        throw new Error('Original transaction message count or destination mismatch');
      const finalizedReplies = await waitForFinalizedReplies(api, blockHash, queued, deadline);
      const replies = queued.map(({ destination, id }, index) => {
        const programId = destination.toHex();
        const messageId = id.toHex();
        const program = programs.find((candidate) => candidate?.programId.toLowerCase() === programId.toLowerCase());
        if (!program) throw new Error(`Program is not found: ${programId}`);
        try {
          const { details, payload } = finalizedReplies[index];
          throwOnErrorReply(details.unwrap().code, payload, api.specVersion, program.registry);
          return { programId, messageId, payload: payload.toU8a(true) };
        } catch (error) {
          throw new Error(`Message ${messageId}: ${error instanceof Error ? error.message : String(error)}`);
        }
      });
      return { blockHash, txHash, replies };
    } catch (error) {
      throw new Error(
        `Vara source transaction ${txHash} at ${blockHash}: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  };

  return useMutation({ mutationFn: signAndSend });
}

export { useSignAndSend };
