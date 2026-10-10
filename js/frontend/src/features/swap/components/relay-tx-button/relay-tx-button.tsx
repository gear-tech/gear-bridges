import { HexString } from '@gear-js/api';
import type { InboundTokenEffect, OutboundEffect } from '@gear-js/bridge';
import { DEFAULT_ERROR_OPTIONS, DEFAULT_SUCCESS_OPTIONS, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';
import { captureException } from '@sentry/react';

import { Tooltip } from '@/components';
import { useNetworkType } from '@/context/network-type';
import { useAccountsConnection, useModal } from '@/hooks';
import { getErrorMessage, isUndefined, logger } from '@/utils';

import { useIsEthRelayAvailable, useIsVaraRelayAvailable, useRelayEthTx, useRelayVaraTx } from '../../hooks';

type VaraProps = {
  nonce: bigint;
  blockNumber: string;
  onFinalized: () => void;
  expectedEffect: OutboundEffect;
};

function RelayVaraTxButton({ nonce, blockNumber, ...props }: VaraProps) {
  const { isAnyAccount, isEthAccount } = useAccountsConnection();
  const { open: openEthModal } = useAppKit();

  const alert = useAlert();

  const { data: isAvailable } = useIsVaraRelayAvailable(blockNumber);
  const { mutate, isPending } = useRelayVaraTx(nonce, BigInt(blockNumber), props.expectedEffect);

  const handleClick = async () => {
    if (!isEthAccount) return openEthModal();

    const alertId = alert.loading('Relaying Vara transaction...');
    const onLog = (message: string) => alert.update(alertId, message);

    const onFinalized = () => {
      props.onFinalized();
      alert.update(alertId, 'Original token transfer finalized on Ethereum', DEFAULT_SUCCESS_OPTIONS);
    };

    const onError = (error: Error) => {
      logger.error('Vara -> Eth relay', error);
      alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS);
      captureException(error, { tags: { feature: 'manual-tx-relay' } });
    };

    mutate({ onLog, onFinalized, onError });
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
        <p>A network fee is required. {!isEthAccount && 'Wallet connection will be requested.'}</p>
      </>
    );
  };

  if (!isAnyAccount) return;

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
  onFinalized: () => void;
  expectedEffect: InboundTokenEffect;
};

function RelayEthTxButton({ txHash, blockNumber, ...props }: EthProps) {
  const { NETWORK_PRESET } = useNetworkType();
  const hold = NETWORK_PRESET.INBOUND_PROOF_PROFILE.hold;
  const { isAnyAccount, isVaraAccount } = useAccountsConnection();
  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();

  const alert = useAlert();

  const { data: isAvailable } = useIsEthRelayAvailable(blockNumber);
  const { mutate, isPending } = useRelayEthTx(txHash, props.expectedEffect);

  const handleClick = () => {
    if (!isVaraAccount) return openSubstrateModal();

    const alertId = alert.loading('Relaying Ethereum transaction...');

    const onLog = (message: string) => alert.update(alertId, message);

    const onFinalized = () => {
      alert.update(alertId, 'Original consumer and token effect finalized on Vara', DEFAULT_SUCCESS_OPTIONS);
      props.onFinalized();
    };

    const onError = (error: Error) => {
      logger.error('Eth -> Vara relay', error);
      alert.update(alertId, getErrorMessage(error), DEFAULT_ERROR_OPTIONS);
    };

    mutate({ onLog, onFinalized, onError });
  };

  const renderTooltipText = () => {
    if (hold) return <p role="status">{hold} This claim remains pending.</p>;
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
        <p>A network fee is required. {!isVaraAccount && 'Wallet connection will be requested.'}</p>
      </>
    );
  };

  if (!isAnyAccount) return;

  return (
    <>
      <Tooltip value={renderTooltipText()}>
        {/* wrapping into span to preserve tooltip while button is disabled */}
        <span>
          <Button
            text="Claim Manually"
            size="x-small"
            onClick={handleClick}
            isLoading={!hold && (isPending || isUndefined(isAvailable))}
            disabled={Boolean(hold) || !isAvailable}
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
