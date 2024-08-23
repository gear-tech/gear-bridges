import { useAlert } from '@gear-js/react-hooks';
import { BaseError } from 'viem';
import { useWriteContract } from 'wagmi';

import { isUndefined, logger } from '@/utils';

import { FUNCTION_NAME, ABI } from '../../consts';
import { Config, Contract, FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit({ address: bridgeAddress }: Contract, { fee, ftAddress }: Config) {
  const alert = useAlert();
  const { writeContract, isPending } = useWriteContract();
  const approve = useApprove(ftAddress);

  const onSubmit = ({ amount: _amount, expectedAmount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (isUndefined(fee.value)) throw new Error('Fee is not defined');

    const address = bridgeAddress;
    const amount = expectedAmount;

    const isNativeToken = !ftAddress;
    const value = isNativeToken ? _amount : fee.value;

    const transit = () => {
      logger.info(
        FUNCTION_NAME.TRANSIT,
        `\naddress: ${address}\nargs: [${accountAddress}, ${amount}]\nvalue: ${value}\nisNativeToken: ${isNativeToken}`,
      );

      writeContract(
        {
          abi: ABI,
          address,
          functionName: FUNCTION_NAME.TRANSIT,
          args: [accountAddress, amount],
          value,
        },
        {
          onSuccess,
          onError: (error) => {
            const errorMessage = (error as BaseError).shortMessage || error.message;

            logger.error(FUNCTION_NAME.TRANSIT, error);
            alert.error(errorMessage);
          },
        },
      );
    };

    return isNativeToken ? transit() : approve.write(address, amount, transit);
  };

  const isSubmitting = approve.isPending || isPending;

  return { onSubmit, isSubmitting };
}

export { useHandleEthSubmit };
