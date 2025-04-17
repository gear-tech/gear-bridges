import { HexString } from '@gear-js/api';
import { getVaraAddress, useAccount, useAlert, useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';
import { Button, Modal } from '@gear-js/vara-ui';
import { isUndefined } from '@polkadot/util';
import { useQueryClient } from '@tanstack/react-query';
import { JSX } from 'react';

import { Address, CopyButton, FeeAndTimeFooter, FormattedBalance, LinkButton } from '@/components';
import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '@/features/swap/consts';
import { useEthFee, useVaraFee } from '@/features/swap/hooks';
import { useTokens } from '@/hooks';
import { cx, getErrorMessage } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK_SVG } from '../../consts';
import { Network, Status, Transfer } from '../../types';
import { TransactionDate } from '../transaction-date';
import { TransactionStatus } from '../transaction-status';

import styles from './transaction-modal.module.scss';

type Props = Pick<
  Transfer,
  'amount' | 'destination' | 'source' | 'sourceNetwork' | 'destNetwork' | 'sender' | 'receiver'
> & {
  txHash?: Transfer['txHash'];
  timestamp?: Transfer['timestamp'];
  status?: Transfer['status'];
  nonce?: Transfer['nonce'];
  close: () => void;
  renderProgressBar?: () => JSX.Element;
};

// TODO: reuse hook from @features/swap
function usePayFee() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  return useSendProgramTransaction({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'payFees',
  });
}

function TransactionModal({
  status,
  txHash,
  sourceNetwork,
  destNetwork,
  timestamp,
  amount,
  source,
  destination,
  sender,
  receiver,
  nonce,
  renderProgressBar,
  close,
}: Props) {
  const { decimals, symbols } = useTokens();
  const isGearNetwork = sourceNetwork === Network.Gear;

  const { fee: varaFee } = useVaraFee();
  const { fee: ethFee } = useEthFee();
  const fee = isGearNetwork ? varaFee : ethFee;

  const { account } = useAccount();
  const payFee = usePayFee();
  const alert = useAlert();
  const queryClient = useQueryClient();
  const isPayFeeButtonVisible = nonce && account?.decodedAddress === sender && status === Status.Pending;

  const explorerUrl = `${isGearNetwork ? 'https://vara.subscan.io/extrinsic' : 'https://etherscan.io/tx'}/${txHash}`;

  const SourceNetworkSVG = NETWORK_SVG[sourceNetwork];
  const DestinationNetworkSVG = NETWORK_SVG[destNetwork];

  const sourceSymbol = symbols?.[source as HexString] || 'Unit';
  const destinationSymbol = symbols?.[destination as HexString] || 'Unit';

  const formattedSenderAddress = isGearNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isGearNetwork ? receiver : getVaraAddress(receiver);

  const handlePayFeeButtonClick = () => {
    if (!nonce) throw new Error('Nonce is not found');
    if (isUndefined(fee.value)) throw new Error('Fee is not found');

    const nonceHex = `0x${nonce.padStart(64, '0')}`;

    payFee
      .sendTransactionAsync({ args: [nonceHex], value: fee.value })
      .then(() => {
        close();
        alert.success('Fee paid successfully');

        return queryClient.invalidateQueries({ queryKey: ['transactions'] });
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));
  };

  return (
    <Modal
      heading="Transaction Details"
      headerAddon={status && <TransactionStatus status={status} />}
      close={close}
      maxWidth="large">
      {(txHash || timestamp) && (
        <header className={styles.header}>
          {txHash && (
            <p className={styles.transactionHash}>
              <a href={explorerUrl} target="_blank" rel="noreferrer">
                <Address value={txHash} />
              </a>

              <CopyButton value={txHash} />
            </p>
          )}

          {timestamp && <TransactionDate timestamp={timestamp} className={styles.date} />}
        </header>
      )}

      <div className={cx(styles.pairs, renderProgressBar && styles.loading)}>
        <span className={styles.tx}>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={decimals?.[source as HexString] ?? 0}
            symbol={sourceSymbol}
            className={styles.amount}
          />

          <span className={styles.label}>on</span>

          <span className={styles.network}>
            <SourceNetworkSVG />
            {isGearNetwork ? 'Vara' : sourceNetwork}
          </span>
        </span>

        <ArrowSVG className={styles.arrowSvg} />

        <span className={styles.tx}>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={decimals?.[source as HexString] ?? 0}
            symbol={destinationSymbol}
            className={styles.amount}
          />

          <span className={styles.label}>on</span>

          <span className={styles.network}>
            <DestinationNetworkSVG className={styles.networkSvg} />
            {isGearNetwork ? destNetwork : 'Vara'}
          </span>
        </span>

        <span className={styles.address}>
          <span className={styles.label}>From</span>
          <Address value={formattedSenderAddress} className={styles.value} />
        </span>

        <ArrowSVG className={styles.arrowSvg} />

        <span className={styles.address}>
          <span className={styles.label}>To</span>
          <Address value={formattedReceiverAddress} className={styles.value} />
        </span>
      </div>

      {renderProgressBar?.()}

      <footer className={styles.footer}>
        <FeeAndTimeFooter fee={fee.formattedValue} symbol={isGearNetwork ? 'VARA' : 'ETH'} />

        {(txHash || isPayFeeButtonVisible) && (
          <div className={styles.buttons}>
            {txHash && (
              <LinkButton type="external" to={explorerUrl} text="View in Explorer" color="grey" size="small" block />
            )}

            {isPayFeeButtonVisible && (
              <Button
                text="Pay Fee"
                size="small"
                isLoading={payFee.isPending}
                onClick={handlePayFeeButtonClick}
                block
              />
            )}
          </div>
        )}
      </footer>
    </Modal>
  );
}

export { TransactionModal };
