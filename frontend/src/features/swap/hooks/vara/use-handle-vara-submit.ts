import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery, useSendProgramTransaction } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';

import { VftProgram, WrappedVaraProgram, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FUNCTION_NAME, QUERY_NAME, SERVICE_NAME } from '../../consts/vara';
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

function useTransferGasLimit() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  return useProgramQuery({
    program,
    serviceName: SERVICE_NAME.BRIDGING_PAYMENT,
    functionName: QUERY_NAME.GET_CONFIG,
    args: [],

    query: {
      select: (data) => {
        const gasLimit =
          BigInt(data.gas_for_reply_deposit) +
          BigInt(data.gas_to_send_request_to_vft_manager) +
          BigInt(data.gas_for_request_to_vft_manager_msg);

        const increasePercent = 3n;

        return gasLimit + (gasLimit * increasePercent) / 100n;
      },
    },
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
  const { data: vftManagerAddress, isLoading: isVftManagerAddressLoading } = useVFTManagerAddress();
  const { data: gasLimit, isLoading: isGasLimitLoading } = useTransferGasLimit();
  const isLoading = isVftManagerAddressLoading || isGasLimitLoading;

  const sendBridgingPaymentRequest = (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');
    if (isUndefined(gasLimit)) throw new Error('Gas limit is not found');

    return bridgingPaymentRequest.sendTransactionAsync({
      gasLimit,
      args: [amount, accountAddress, ftAddress],
      value: feeValue,
    });
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');
    if (!vftManagerAddress) throw new Error('VFT manager address is not found');
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (isUndefined(allowance)) throw new Error('Allowance is not found');
    if (isUndefined(ftBalance)) throw new Error('FT balance is not found');

    if (ftAddress === WRAPPED_VARA_CONTRACT_ADDRESS && amount > ftBalance) {
      await mint.sendTransactionAsync({ args: [], value: amount - ftBalance });
    } else {
      mint.reset();
    }

    if (amount > allowance) {
      await vftApprove.sendTransactionAsync({ args: [vftManagerAddress, amount] });
    } else {
      vftApprove.reset();
    }

    return sendBridgingPaymentRequest(amount, accountAddress);
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, { ...vftApprove, isLoading }, mint] as const;
}

export { useHandleVaraSubmit };
