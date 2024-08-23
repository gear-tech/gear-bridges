import { isUndefined, logger } from '@/utils';

import { Config, Contract, FormattedValues } from '../../types';

import { useSendMessage } from './use-send-message';

function useHandleVaraSubmit({ address, metadata }: Contract, { fee, ftAddress }: Config) {
  const isNativeToken = !ftAddress;

  const { sendMessage, isPending } = useSendMessage(address, metadata, {
    disableAlerts: true,
    isMaxGasLimit: !isNativeToken,
  });

  const onSubmit = ({ amount: _amount, expectedAmount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (isUndefined(fee.value)) throw new Error('Fee is not defined');

    const amount = String(expectedAmount); // TODO: fix after @gear-js/react-hooks bigint support
    const recipient = accountAddress;
    const payload = { TransitVaraToEth: { amount, recipient } };

    const value = String(isNativeToken ? _amount : fee.value); // TODO: fix after @gear-js/react-hooks bigint support

    logger.info(
      'TransitVaraToEth',
      `\nprogramId:${address}\namount: ${amount}\nrecipient: ${recipient}\nvalue: ${value}\nisNativeToken: ${isNativeToken}`,
    );

    sendMessage({ payload, value, onSuccess });
  };

  const isSubmitting = isPending;

  return { onSubmit, isSubmitting };
}

export { useHandleVaraSubmit };
