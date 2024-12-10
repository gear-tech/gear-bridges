import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { useLayoutEffect } from 'react';
import { formatUnits } from 'viem';

import { CopyButton, FeeAndTimeFooter, LinkButton, TruncatedText } from '@/components';
import { useEthFee, useVaraFee } from '@/features/swap/hooks';
import { useTokens } from '@/hooks';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK_SVG } from '../../consts';
import { Network, Transfer } from '../../types';
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
  close: () => void;
  renderProgressBar?: () => JSX.Element;
};

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
  renderProgressBar,
  close,
}: Props) {
  const { decimals, symbols } = useTokens();
  const isGearNetwork = sourceNetwork === Network.Gear;

  const { fee: varaFee } = useVaraFee();
  const { fee: ethFee } = useEthFee();
  const fee = isGearNetwork ? varaFee : ethFee;

  const explorerUrl = `${isGearNetwork ? 'https://vara.subscan.io/extrinsic' : 'https://etherscan.io/tx'}/${txHash}`;

  const SourceNetworkSVG = NETWORK_SVG[sourceNetwork];
  const DestinationNetworkSVG = NETWORK_SVG[destNetwork];

  const sourceSymbol = symbols?.[source as HexString] || 'Unit';
  const destinationSymbol = symbols?.[destination as HexString] || 'Unit';

  const formattedAmount = formatUnits(BigInt(amount), decimals?.[source as HexString] ?? 0);

  const formattedSenderAddress = isGearNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isGearNetwork ? receiver : getVaraAddress(receiver);

  useLayoutEffect(() => {
    // TODO: monkey patch, update after @gear-js/vara-ui is updated to support different modal sizes
    setTimeout(() => {
      const modalElement = document.querySelector('#modal-root > div > div');
      modalElement?.classList.add(styles.modal);
    }, 0);
  }, []);

  const renderHeading = () => (
    <>
      Transaction Details
      {status && <TransactionStatus status={status} />}
    </>
  );

  return (
    // TODO: remove assertion after @gear-js/vara-ui update
    <Modal heading={renderHeading() as unknown as string} close={close}>
      {(txHash || timestamp) && (
        <header className={styles.header}>
          {txHash && (
            <p className={styles.transactionHash}>
              <a href={explorerUrl} target="_blank" rel="noreferrer">
                <TruncatedText value={txHash} />
              </a>

              <CopyButton value={txHash} />
            </p>
          )}

          {timestamp && <TransactionDate timestamp={timestamp} />}
        </header>
      )}

      <p className={cx(styles.pairs, renderProgressBar && styles.loading)}>
        <span className={styles.tx}>
          <span className={styles.amount}>
            {formattedAmount} {sourceSymbol}
          </span>

          <span className={styles.label}>on</span>

          <span className={styles.network}>
            <SourceNetworkSVG />
            {isGearNetwork ? 'Vara' : sourceNetwork}
          </span>
        </span>

        <ArrowSVG className={styles.arrowSvg} />

        <span className={styles.tx}>
          <span className={styles.amount}>
            {formattedAmount} {destinationSymbol}
          </span>

          <span className={styles.label}>on</span>

          <span className={styles.network}>
            <DestinationNetworkSVG className={styles.networkSvg} />
            {isGearNetwork ? destNetwork : 'Vara'}
          </span>
        </span>

        <span className={styles.address}>
          <span className={styles.label}>From</span>
          <TruncatedText value={formattedSenderAddress} className={styles.value} />
        </span>

        <ArrowSVG className={styles.arrowSvg} />

        <span className={styles.address}>
          <span className={styles.label}>To</span>
          <TruncatedText value={formattedReceiverAddress} className={styles.value} />
        </span>
      </p>

      {renderProgressBar?.()}

      <footer className={styles.footer}>
        <FeeAndTimeFooter fee={fee.formattedValue} symbol={isGearNetwork ? 'VARA' : 'ETH'} />

        {txHash && (
          <LinkButton type="external" to={explorerUrl} text="View in Explorer" color="grey" size="small" block />
        )}
      </footer>
    </Modal>
  );
}

export { TransactionModal };
