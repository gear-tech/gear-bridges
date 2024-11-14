import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { BaseError } from 'viem';
import { useWriteContract } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';

import { logger } from '@/utils';

import { BRIDGING_PAYMENT_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(ftAddress: HexString | undefined, fee: bigint | undefined) {
  const { writeContract, isPending } = useWriteContract();
  const approve = useApprove(ftAddress);
  const alert = useAlert();

  const onError = (error: WriteContractErrorType) => {
    const errorMessage = (error as BaseError).shortMessage || error.message;

    logger.error(FUNCTION_NAME.DEPOSIT, error);
    alert.error(errorMessage);
  };

  const onSubmit = ({ amount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');
    if (!fee) throw new Error('Fee is not defined');

    const requestBridging = () =>
      writeContract(
        {
          abi: BRIDGING_PAYMENT_ABI,
          address: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
          functionName: 'requestBridging',
          args: [ftAddress, amount, accountAddress],
          value: fee,
        },
        { onSuccess, onError },
      );

    approve.write(amount, requestBridging);
  };

  const { isLoading } = approve;
  const isSubmitting = approve.isPending || isPending;

  return { onSubmit, isSubmitting, isLoading };
}

export { useHandleEthSubmit };
