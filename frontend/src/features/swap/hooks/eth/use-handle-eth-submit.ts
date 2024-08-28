import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { BaseError } from 'viem';
import { useWriteContract } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';

import { logger } from '@/utils';

import { FUNCTION_NAME, ERC20_TREASURY_ABI, ERC20_TREASURY_CONTRACT_ADDRESS } from '../../consts';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(ftAddress: HexString | undefined) {
  const alert = useAlert();
  const { writeContract, isPending } = useWriteContract();
  const approve = useApprove(ftAddress);

  const onError = (error: WriteContractErrorType) => {
    const errorMessage = (error as BaseError).shortMessage || error.message;

    logger.error(FUNCTION_NAME.DEPOSIT, error);
    alert.error(errorMessage);
  };

  const onSubmit = ({ amount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');

    const address = ERC20_TREASURY_CONTRACT_ADDRESS;

    const deposit = () => {
      logger.info(
        FUNCTION_NAME.DEPOSIT,
        `\naddress: ${address}\nargs: [ftAddress: ${ftAddress}, amount: ${amount}, accountAddress: ${accountAddress}]`,
      );

      writeContract(
        {
          abi: ERC20_TREASURY_ABI,
          address,
          functionName: FUNCTION_NAME.DEPOSIT,
          args: [ftAddress, amount, accountAddress],
        },
        { onSuccess, onError },
      );
    };

    approve.write(amount, deposit);
  };

  const isSubmitting = approve.isPending || isPending;

  return { onSubmit, isSubmitting };
}

export { useHandleEthSubmit };
