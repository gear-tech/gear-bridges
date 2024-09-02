import { HexString } from '@gear-js/api';
import { useAlert, useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';

import { isUndefined, logger } from '@/utils';

import { BRIDGING_PAYMENT_CONTRACT_ADDRESS, BridgingPaymentProgram, VftProgram } from '../../consts';
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
    functionName: FUNCTION_NAME.REQUEST_TO_GATEWAY,
  });
}

function useSendVftApprove(ftAddress: HexString | undefined) {
  const { data: program } = useProgram({
    library: VftProgram,
    id: ftAddress,
    query: { enabled: Boolean(ftAddress) },
  });

  return useSendProgramTransaction({
    program,
    serviceName: SERVICE_NAME.VFT,
    functionName: FUNCTION_NAME.APPROVE,
  });
}

function useHandleVaraSubmit(ftAddress: HexString | undefined, feeValue: bigint | undefined) {
  const alert = useAlert();

  const bridgingPaymentRequest = useSendBridgingPaymentRequest();
  const vftApprove = useSendVftApprove(ftAddress);

  const onSubmit = async ({ amount, accountAddress }: FormattedValues, onSuccess: () => void) => {
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (!ftAddress) throw new Error('Fungible token address is not found');

    const recipient = accountAddress;
    const value = feeValue;

    logger.info(
      'TransitVaraToEth',
      `\nprogramId:${ftAddress}\namount: ${amount}\nrecipient: ${recipient}\nvalue: ${value}\nfee: ${feeValue}`,
    );

    try {
      await vftApprove.sendTransactionAsync({ args: [BRIDGING_PAYMENT_CONTRACT_ADDRESS, amount] });

      await bridgingPaymentRequest.sendTransactionAsync({
        args: [amount, recipient, ftAddress],
        gasLimit: BigInt(350000000000),
        value,
      });

      onSuccess();
    } catch (error) {
      alert.error(error instanceof Error ? error.message : String(error));
    }
  };

  const isSubmitting = bridgingPaymentRequest.isPending || vftApprove.isPending;

  return { onSubmit, isSubmitting };
}

export { useHandleVaraSubmit };
