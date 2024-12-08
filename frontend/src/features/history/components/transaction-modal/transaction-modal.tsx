import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { useLayoutEffect } from 'react';
import { formatUnits } from 'viem';

import GasSVG from '@/assets/gas.svg?react';
import { CopyButton, LinkButton, TruncatedText } from '@/components';
import { useEthFee, useVaraFee } from '@/features/swap/hooks';
import { useTokens } from '@/hooks';

import ArrowSVG from '../../assets/arrow.svg?react';
import ClockSVG from '../../assets/clock.svg?react';
import { NETWORK_SVG } from '../../consts';
import { Network, Transfer } from '../../types';
import { TransactionDate } from '../transaction-date';
import { TransactionStatus } from '../transaction-status';

import styles from './transaction-modal.module.scss';

type Props = Pick<
  Transfer,
  | 'amount'
  | 'destination'
  | 'source'
  | 'status'
  | 'timestamp'
  | 'txHash'
  | 'sourceNetwork'
  | 'destNetwork'
  | 'sender'
  | 'receiver'
> & {
  close: () => void;
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
      <TransactionStatus status={status} />
    </>
  );

  return (
    // TODO: remove assertion after @gear-js/vara-ui update
    <Modal heading={renderHeading() as unknown as string} close={close}>
      <header className={styles.header}>
        <p className={styles.transactionHash}>
          <a href={explorerUrl} target="_blank" rel="noreferrer">
            <TruncatedText value={txHash} />
          </a>

          <CopyButton value={txHash} />
        </p>

        <TransactionDate timestamp={timestamp} isCompact />
      </header>

      <p className={styles.pairs}>
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

      <footer>
        <div className={styles.stats}>
          <p className={styles.stat}>
            <span>Paid Fee:</span>

            <span className={styles.value}>
              <GasSVG />
              {`${fee.formattedValue} ${isGearNetwork ? 'VARA' : 'ETH'}`}
            </span>
          </p>

          <p className={styles.stat}>
            <span>Bridge Time:</span>

            <span className={styles.value}>
              <ClockSVG />
              ~30 mins
            </span>
          </p>
        </div>

        <LinkButton type="external" to={explorerUrl} text="View in Explorer" color="grey" size="small" block />
      </footer>
    </Modal>
  );
}

export { TransactionModal };
