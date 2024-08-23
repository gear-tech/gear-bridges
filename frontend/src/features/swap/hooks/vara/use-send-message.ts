import { HexString, ProgramMetadata } from '@gear-js/api';
import {
  SendMessageWithGasOptions,
  UseSendMessageWithGasOptions,
  useSendMessageWithGas as useGearSendMessageWithGas,
} from '@gear-js/react-hooks';

import { useLoading } from '@/hooks';

function useSendMessage(
  programId: HexString,
  metadata: ProgramMetadata | undefined,
  options?: UseSendMessageWithGasOptions,
) {
  const sendMessage = useGearSendMessageWithGas(programId, metadata, options);

  const [isLoading, enableLoading, disableLoading] = useLoading();

  return {
    sendMessage: (args: SendMessageWithGasOptions) => {
      enableLoading();

      const onSuccess = (messageId: HexString) => {
        args.onSuccess?.(messageId);
        disableLoading();
      };

      const onError = () => {
        args.onError?.();
        disableLoading();
      };

      sendMessage({ ...args, onSuccess, onError });
    },

    isPending: isLoading,
  };
}

export { useSendMessage };
