import { HexString } from '@gear-js/api';

import { isUndefined, logger } from '@/utils';

import { Contract, FormattedValues } from '../../types';

import { useSendMessage } from './use-send-message';

function useHandleVaraSubmit({ address, sails }: Contract, ftAddress: HexString | undefined) {
  const isNativeToken = !ftAddress;

  // For fungble token contracts gas calculation does not work cuz contracts check the amount of gas applied
  const gasLimit = isNativeToken ? undefined : BigInt(100_000_000_000);
  const { sendMessage, isPending } = useSendMessage(gasLimit);

  const onSubmit = ({ amount: _amount, expectedAmount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (isUndefined(sails)) throw new Error('Sails is not defined');

    const fee = { value: BigInt(0) };

    const amount = expectedAmount;
    const recipient = accountAddress;

    const value = isNativeToken ? _amount + fee.value : fee.value;

    logger.info(
      'TransitVaraToEth',
      `\nprogramId:${address}\namount: ${amount}\nrecipient: ${recipient}\nvalue: ${value}\nisNativeToken: ${isNativeToken}\nfee: ${fee.value}`,
    );

    const { TransitVaraToEth } = sails.services.VaraBridge.functions;
    const transaction = TransitVaraToEth<null>(fee.value, recipient, amount);

    return sendMessage(transaction, value, { onSuccess });
  };

  const isSubmitting = isPending;

  return { onSubmit, isSubmitting };
}

export { useHandleVaraSubmit };
