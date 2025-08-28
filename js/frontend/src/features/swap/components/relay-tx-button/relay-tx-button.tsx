import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import { getErrorMessage } from '@/utils';

import { useRelayEthTx, useRelayVaraTx } from '../../hooks';

type VaraProps = {
  nonce: bigint | HexString;
  blockNumber: bigint;
};

function RelayVaraTxButton({ nonce, blockNumber }: VaraProps) {
  const alert = useAlert();
  const { mutateAsync, isPending } = useRelayVaraTx(nonce, blockNumber);

  const handleClick = () =>
    mutateAsync()
      .then((result) => {
        console.log(result);
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));

  return <Button text="Manual Claim" size="x-small" onClick={handleClick} isLoading={isPending} />;
}

type EthProps = {
  txHash: HexString;
};

function RelayEthTxButton({ txHash }: EthProps) {
  const alert = useAlert();
  const { mutateAsync, isPending } = useRelayEthTx(txHash);

  const handleClick = () =>
    mutateAsync()
      .then((result) => {
        console.log(result);
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));

  return <Button text="Manual Claim" size="x-small" onClick={handleClick} isLoading={isPending} />;
}

const RelayTxButton = {
  Vara: RelayVaraTxButton,
  Eth: RelayEthTxButton,
};

export { RelayTxButton };
