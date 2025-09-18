import { HexString } from '@gear-js/api';
import { DEFAULT_ERROR_OPTIONS, DEFAULT_SUCCESS_OPTIONS, useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import { useEthAccount, useModal } from '@/hooks';
import { getErrorMessage } from '@/utils';

import { useIsEthRelayAvailable, useIsVaraRelayAvailable, useRelayEthTx, useRelayVaraTx } from '../../hooks';

type VaraProps = {
  sender: string;
  nonce: HexString;
  blockNumber: string;
  onSuccess: () => void;
};

function RelayVaraTxButton({ sender, nonce, blockNumber, onSuccess }: VaraProps) {
  const { account } = useAccount();
  const isOwner = account?.decodedAddress === sender;

  const ethAccount = useEthAccount();
  const { open: openEthModal } = useAppKit();

  const alert = useAlert();

  const { data: isAvailable } = useIsVaraRelayAvailable(blockNumber);
  const { mutateAsync, isPending } = useRelayVaraTx(nonce, BigInt(blockNumber));

  const handleClick = () => {
    if (!ethAccount.address) return openEthModal();

    const alertId = alert.loading('Relaying Vara transaction...');
    const onLog = (message: string) => alert.update(alertId, message);

    mutateAsync(onLog)
      .then(() => {
        onSuccess();
        alert.update(alertId, 'Vara transaction relayed successfully', DEFAULT_SUCCESS_OPTIONS);
      })
      .catch((error: Error) => alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS));
  };

  if (account ? !isOwner : !ethAccount.address) return;

  return (
    <Button text="Claim Manually" size="x-small" onClick={handleClick} isLoading={isPending} disabled={!isAvailable} />
  );
}

type EthProps = {
  sender: string;
  blockNumber: bigint;
  txHash: HexString;
  onSuccess: () => void;
};

function RelayEthTxButton({ sender, txHash, blockNumber, onSuccess }: EthProps) {
  const { account } = useAccount();
  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();

  const ethAccount = useEthAccount();
  const isOwner = ethAccount.address?.toLowerCase() === sender;

  const alert = useAlert();

  const { data: isAvailable } = useIsEthRelayAvailable(blockNumber);
  const { mutateAsync, isPending } = useRelayEthTx(txHash);

  const handleClick = () => {
    if (!account) return openSubstrateModal();

    const alertId = alert.loading('Relaying Ethereum transaction...');
    const onLog = (message: string) => alert.update(alertId, message);

    mutateAsync(onLog)
      .then(() => {
        onSuccess();
        alert.update(alertId, 'Ethereum transaction relayed successfully', DEFAULT_SUCCESS_OPTIONS);
      })
      .catch((error: Error) => alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS));
  };

  if (ethAccount.address ? !isOwner : !account) return;

  return (
    <>
      <Button
        text="Claim Manually"
        size="x-small"
        onClick={handleClick}
        isLoading={isPending}
        disabled={!isAvailable}
      />

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

const RelayTxButton = {
  Vara: RelayVaraTxButton,
  Eth: RelayEthTxButton,
};

export { RelayTxButton };
