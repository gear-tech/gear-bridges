import { HexString } from '@gear-js/api';
import { DEFAULT_ERROR_OPTIONS, DEFAULT_SUCCESS_OPTIONS, useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import { Tooltip } from '@/components';
import { useEthAccount, useModal } from '@/hooks';
import { getErrorMessage, isUndefined, logger } from '@/utils';

import { useIsEthRelayAvailable, useIsVaraRelayAvailable, useRelayEthTx, useRelayVaraTx } from '../../hooks';

type VaraProps = {
  nonce: HexString;
  blockNumber: string;
  onReceipt: () => void;
  onConfirmation: () => void;
};

function RelayVaraTxButton({ nonce, blockNumber, onConfirmation, ...props }: VaraProps) {
  const { account } = useAccount();

  const ethAccount = useEthAccount();
  const { open: openEthModal } = useAppKit();

  const alert = useAlert();

  const { data: isAvailable } = useIsVaraRelayAvailable(blockNumber);
  const { mutate, isPending } = useRelayVaraTx(nonce, BigInt(blockNumber));

  const handleClick = async () => {
    if (!ethAccount.address) return openEthModal();

    const alertId = alert.loading('Relaying Vara transaction...');
    const onLog = (message: string) => alert.update(alertId, message);

    const onReceipt = () => {
      props.onReceipt();
      alert.update(alertId, 'Vara transaction relayed successfully', DEFAULT_SUCCESS_OPTIONS);
    };

    const onError = (error: Error) => {
      logger.error('Vara -> Eth relay', error);
      alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS);
    };

    mutate({ onLog, onReceipt, onConfirmation, onError });
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

  if (!account && !ethAccount.address) return;

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
  blockNumber: bigint;
  txHash: HexString;
  onInBlock: () => void;
  onFinalization: () => void;
};

function RelayEthTxButton({ txHash, blockNumber, onFinalization, ...props }: EthProps) {
  const { account } = useAccount();
  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();

  const ethAccount = useEthAccount();
  const alert = useAlert();

  const { data: isAvailable } = useIsEthRelayAvailable(blockNumber);
  const { mutate, isPending } = useRelayEthTx(txHash);

  const handleClick = () => {
    if (!account) return openSubstrateModal();

    const alertId = alert.loading('Relaying Ethereum transaction...');

    const onLog = (message: string) => alert.update(alertId, message);

    const onInBlock = () => {
      alert.update(alertId, 'Ethereum transaction relayed successfully', DEFAULT_SUCCESS_OPTIONS);
      props.onInBlock();
    };

    const onError = (error: Error) => {
      logger.error('Eth -> Vara relay', error);
      alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS);
    };

    mutate({ onLog, onInBlock, onFinalization, onError });
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
        <p>A network fee is required. {!account && 'Wallet connection will be requested.'}</p>
      </>
    );
  };

  if (!account && !ethAccount.address) return;

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
