import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { useWriteContract } from 'wagmi';

import { isUndefined } from '@/utils';

import { BRIDGING_PAYMENT_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FormattedValues, UseFTAllowance } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(
  ftAddress: HexString | undefined,
  fee: bigint | undefined,
  allowance: ReturnType<UseFTAllowance>,
) {
  const { writeContractAsync } = useWriteContract();
  const approve = useApprove(ftAddress);

  const requestBridging = (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');
    if (!fee) throw new Error('Fee is not defined');

    return writeContractAsync({
      abi: BRIDGING_PAYMENT_ABI,
      address: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
      functionName: 'requestBridging',
      args: [ftAddress, amount, accountAddress],
      value: fee,
    });
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    if (isUndefined(allowance.data)) throw new Error('Allowance is not defined');

    if (amount > allowance.data) {
      await approve.mutateAsync(amount);
      await allowance.refetch(); // TODO: replace with queryClient.setQueryData after @gear-js/react-hooks update to return QueryKey
    }

    return requestBridging(amount, accountAddress);
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, approve] as const;
}

export { useHandleEthSubmit };
