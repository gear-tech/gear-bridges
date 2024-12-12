import {
  useProgram,
  usePrepareProgramTransaction,
  useSendProgramTransaction,
  useProgramQuery,
} from '@gear-js/react-hooks';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS, SERVICE_NAME, QUERY_NAME } from '../../consts';
import { Config } from '../../consts/sails/bridging-payment';

function useTransferGasLimit() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const select = (data: Config) => {
    const gasLimit =
      BigInt(data.gas_for_reply_deposit) +
      BigInt(data.gas_to_send_request_to_vft_manager) +
      BigInt(data.gas_for_request_to_vft_manager_msg);

    const increasePercent = 3n;

    return gasLimit + (gasLimit * increasePercent) / 100n;
  };

  return useProgramQuery({
    program,
    serviceName: SERVICE_NAME.BRIDGING_PAYMENT,
    functionName: QUERY_NAME.GET_CONFIG,
    args: [],
    query: { select },
  });
}

function useTransfer() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const params = { program, serviceName: SERVICE_NAME.BRIDGING_PAYMENT, functionName: 'makeRequest' as const };
  const { prepareTransactionAsync } = usePrepareProgramTransaction(params);
  const send = useSendProgramTransaction(params);

  return { ...send, prepareTransactionAsync };
}

export { useTransferGasLimit, useTransfer };
