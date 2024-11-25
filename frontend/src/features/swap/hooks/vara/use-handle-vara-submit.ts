import { HexString } from '@gear-js/api';
import { useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';

import { BRIDGING_PAYMENT_CONTRACT_ADDRESS, BridgingPaymentProgram, VftProgram } from '@/consts';
import { isUndefined } from '@/utils';

import { FUNCTION_NAME, SERVICE_NAME } from '../../consts/vara';
import { FormattedValues } from '../../types';

function useSendBridgingPaymentRequest() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  return useSendProgramTransaction({
    program,
    serviceName: SERVICE_NAME.BRIDGING_PAYMENT,
    functionName: 'makeRequest',
  });
}

function useSendVftApprove(ftAddress: HexString | undefined) {
  const { data: program } = useProgram({
    library: VftProgram,
    id: ftAddress,
  });

  return useSendProgramTransaction({
    program,
    serviceName: SERVICE_NAME.VFT,
    functionName: FUNCTION_NAME.APPROVE,
  });
}

function useHandleVaraSubmit(
  ftAddress: HexString | undefined,
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
) {
  const bridgingPaymentRequest = useSendBridgingPaymentRequest();
  const vftApprove = useSendVftApprove(ftAddress);

  const sendBridgingPaymentRequest = (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');

    return bridgingPaymentRequest.sendTransactionAsync({
      gasLimit: BigInt(350000000000),
      args: [amount, accountAddress, ftAddress],
      value: feeValue,
    });
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (isUndefined(allowance)) throw new Error('Allowance is not found');

    if (amount > allowance)
      await vftApprove.sendTransactionAsync({ args: [BRIDGING_PAYMENT_CONTRACT_ADDRESS, amount] });

    return sendBridgingPaymentRequest(amount, accountAddress);
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, vftApprove] as const;
}

export { useHandleVaraSubmit };
