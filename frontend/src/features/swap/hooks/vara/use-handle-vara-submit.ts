import { HexString } from '@gear-js/api';
import { useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';

import { VftProgram, WrappedVaraProgram, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FUNCTION_NAME, SERVICE_NAME } from '../../consts/vara';
import { FormattedValues } from '../../types';

import { useVFTManagerAddress } from './use-vft-manager-address';

function useMint() {
  const { data: program } = useProgram({
    library: WrappedVaraProgram,
    id: WRAPPED_VARA_CONTRACT_ADDRESS,
  });

  return useSendProgramTransaction({
    program,
    serviceName: 'tokenizer',
    functionName: 'mint',
  });
}

function useApprove(ftAddress: HexString | undefined) {
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

function useRequestBridging() {
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

function useHandleVaraSubmit(
  ftAddress: HexString | undefined,
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
) {
  const mint = useMint();
  const vftApprove = useApprove(ftAddress);
  const bridgingPaymentRequest = useRequestBridging();
  const { data: vftManagerAddress, isLoading } = useVFTManagerAddress();

  const sendBridgingPaymentRequest = (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');

    return bridgingPaymentRequest.sendTransactionAsync({
      gasLimit: BigInt(350000000000),
      args: [amount, accountAddress, ftAddress],
      value: feeValue,
    });
  };

  const onSubmit = async ({ expectedAmount, accountAddress }: FormattedValues) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');
    if (!vftManagerAddress) throw new Error('VFT manager address is not found');
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (isUndefined(allowance)) throw new Error('Allowance is not found');
    if (isUndefined(ftBalance)) throw new Error('FT balance is not found');

    if (ftAddress === WRAPPED_VARA_CONTRACT_ADDRESS && expectedAmount > ftBalance) {
      await mint.sendTransactionAsync({ args: [], value: expectedAmount - ftBalance });
    } else {
      mint.reset();
    }

    if (expectedAmount > allowance) {
      await vftApprove.sendTransactionAsync({ args: [vftManagerAddress, expectedAmount] });
    } else {
      vftApprove.reset();
    }

    return sendBridgingPaymentRequest(expectedAmount, accountAddress);
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, { ...vftApprove, isLoading }, mint] as const;
}

export { useHandleVaraSubmit };
