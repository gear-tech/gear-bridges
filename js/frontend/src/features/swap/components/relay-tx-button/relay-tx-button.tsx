import { HexString } from '@gear-js/api';
import { DEFAULT_ERROR_OPTIONS, DEFAULT_SUCCESS_OPTIONS, useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import { Tooltip } from '@/components';
import { useEthAccount, useModal } from '@/hooks';
import { getErrorMessage, isUndefined } from '@/utils';

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

  const renderTooltipText = () => {
    if (!isAvailable)
      return (
        <>
          <p>Disabled until finalization completes.</p>
          <p>You&apos;ll be able to claim manually once the block is verified on the Ethereum chain.</p>
        </>
      );

    return (
      <>
        <p>Use your Ethereum chain wallet to claim tokens by yourself.</p>
        <p>A network fee is required. {!ethAccount.address && 'Wallet connection will be requested.'}</p>
      </>
    );
  };

  if (account ? !isOwner : !ethAccount.address) return;

  return (
    <Tooltip value={renderTooltipText()}>
      {/* wrapping into span to preserve tooltip while button is disabled */}
      <span>
        <Button
          text="Claim Manually"
          size="x-small"
          onClick={handleClick}
          isLoading={isPending || isUndefined(isAvailable)}
          disabled={!isAvailable}
          block
        />
      </span>
    </Tooltip>
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

  const renderTooltipText = () => {
    if (!isAvailable)
      return (
        <>
          <p>Disabled until finalization completes.</p>
          <p>You&apos;ll be able to claim manually once the block is verified on the Vara chain.</p>
        </>
      );

    return (
      <>
        <p>Use your Vara chain wallet to claim tokens by yourself.</p>
        <p>A network fee is required.</p>
        <p>A network fee is required. {!account && 'Wallet connection will be requested.'}</p>
      </>
    );
  };

  if (ethAccount.address ? !isOwner : !account) return;

  return (
    <>
      <Tooltip value={renderTooltipText()}>
        {/* wrapping into span to preserve tooltip while button is disabled */}
        <span>
          <Button
            text="Claim Manually"
            size="x-small"
            onClick={handleClick}
            isLoading={isPending || isUndefined(isAvailable)}
            disabled={!isAvailable}
            block
          />
        </span>
      </Tooltip>

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

const RelayTxButton = {
  Vara: RelayVaraTxButton,
  Eth: RelayEthTxButton,
};

export { RelayTxButton };
