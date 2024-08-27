import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { BaseError } from 'viem';
import { useWriteContract } from 'wagmi';

import { logger } from '@/utils';

import { FUNCTION_NAME, ERC20_TREASURY_ABI } from '../../consts';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(bridgeAddress: HexString | undefined, ftAddress: HexString | undefined) {
  const alert = useAlert();
  const { writeContract, isPending } = useWriteContract();
  const approve = useApprove(ftAddress);

  const onSubmit = ({ amount: _amount, expectedAmount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (!ftAddress || !bridgeAddress) throw new Error('Bridge address is not defined');

    const fee = { value: BigInt(0) };

    const address = bridgeAddress;
    const amount = expectedAmount;

    const isNativeToken = !ftAddress;
    const value = isNativeToken ? _amount + fee.value : fee.value;

    const transit = () => {
      logger.info(
        FUNCTION_NAME.DEPOSIT,
        `\naddress: ${address}\nargs: [\nfee: ${fee.value}\namount: ${amount}]` +
          `\nvalue: ${value}\nisNativeToken: ${isNativeToken}`,
      );

      writeContract(
        {
          abi: ERC20_TREASURY_ABI,
          address,
          functionName: FUNCTION_NAME.DEPOSIT,

          args: [ftAddress, amount, accountAddress],
        },
        {
          onSuccess,
          onError: (error) => {
            const errorMessage = (error as BaseError).shortMessage || error.message;

            logger.error(FUNCTION_NAME.DEPOSIT, error);
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
