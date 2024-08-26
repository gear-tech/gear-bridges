import { useAlert } from '@gear-js/react-hooks';
import { BaseError } from 'viem';
import { useWriteContract } from 'wagmi';

import { isUndefined, logger } from '@/utils';

import { FUNCTION_NAME, ABI } from '../../consts';
import { Config, Contract, FeeCalculator, FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(
  { address: bridgeAddress }: Contract,
  { ftAddress }: Config,
  feeCalculatorData?: FeeCalculator,
) {
  const alert = useAlert();
  const { writeContract, isPending } = useWriteContract();
  const approve = useApprove(ftAddress);

  const onSubmit = ({ amount: _amount, expectedAmount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (isUndefined(feeCalculatorData)) throw new Error('FeeCalculatorData is not defined');
    const { fee, mortality, timestamp, signature } = feeCalculatorData || {};

    const address = bridgeAddress;
    const amount = expectedAmount;

    const isNativeToken = !ftAddress;
    const value = isNativeToken ? _amount + fee.value : fee.value;

    const transit = () => {
      logger.info(
        FUNCTION_NAME.TRANSIT,
        `\naddress: ${address}\nargs: [\nfee: ${fee.value}\nmortality: ${mortality}\ntimestamp: ${timestamp}\nsignature: ${signature}\n${accountAddress} \namount: ${amount}\n]` +
          `\nvalue: ${value}\nisNativeToken: ${isNativeToken}`,
      );

      writeContract(
        {
          abi: ABI,
          address,
          functionName: FUNCTION_NAME.TRANSIT,
          args: [fee.value, BigInt(mortality), BigInt(timestamp), signature, accountAddress, amount],
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
