import { HexString } from '@gear-js/api';
import { DEFAULT_ERROR_OPTIONS, DEFAULT_SUCCESS_OPTIONS, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import { getErrorMessage } from '@/utils';

import { useIsEthRelayAvailable, useIsVaraRelayAvailable, useRelayEthTx, useRelayVaraTx } from '../../hooks';

type VaraProps = {
  nonce: HexString;
  blockNumber: string;
};

function RelayVaraTxButton({ nonce, blockNumber }: VaraProps) {
  const alert = useAlert();

  const { data: isAvailable } = useIsVaraRelayAvailable(blockNumber);
  const { mutateAsync, isPending } = useRelayVaraTx(nonce, BigInt(blockNumber));

  const handleClick = () => {
    const alertId = alert.loading('Relaying Vara transaction...');
    const onLog = (message: string) => alert.update(alertId, message);

    mutateAsync(onLog)
      .then(() => alert.update(alertId, 'Vara transaction relayed successfully', DEFAULT_SUCCESS_OPTIONS))
      .catch((error: Error) => alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS));
  };

  return (
    <Button text="Manual Claim" size="x-small" onClick={handleClick} isLoading={isPending} disabled={!isAvailable} />
  );
}

type EthProps = {
  blockNumber: bigint;
  txHash: HexString;
};

function RelayEthTxButton({ txHash, blockNumber }: EthProps) {
  const alert = useAlert();

  const { data: isAvailable } = useIsEthRelayAvailable(blockNumber);
  const { mutateAsync, isPending } = useRelayEthTx(txHash);

  const handleClick = () => {
    const alertId = alert.loading('Relaying Ethereum transaction...');
    const onLog = (message: string) => alert.update(alertId, message);

    mutateAsync(onLog)
      .then(() => alert.update(alertId, 'Ethereum transaction relayed successfully', DEFAULT_SUCCESS_OPTIONS))
      .catch((error: Error) => alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS));
  };

  return (
    <Button text="Manual Claim" size="x-small" onClick={handleClick} isLoading={isPending} disabled={!isAvailable} />
  );
}

const RelayTxButton = {
  Vara: RelayVaraTxButton,
  Eth: RelayEthTxButton,
};

export { RelayTxButton };
