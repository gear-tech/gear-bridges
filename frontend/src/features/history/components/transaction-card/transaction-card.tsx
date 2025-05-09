import { HexString } from '@gear-js/api';

import { Card, CopyButton, Skeleton } from '@/components';
import { useModal } from '@/hooks';
import { getTruncatedText } from '@/utils';

import { Transfer } from '../../types';
import { BlockNumberLink } from '../block-number-link';
import { TransactionDate } from '../transaction-date';
import { TransactionModal } from '../transaction-modal';
import { TransactionPair } from '../transaction-pair';
import { TransactionStatus } from '../transaction-status';

import styles from './transaction-card.module.scss';

type Props = Pick<
  Transfer,
  | 'amount'
  | 'destination'
  | 'source'
  | 'status'
  | 'timestamp'
  | 'sourceNetwork'
  | 'destNetwork'
  | 'txHash'
  | 'sender'
  | 'receiver'
  | 'nonce'
  | 'blockNumber'
> & {
  decimals: Record<HexString, number>;
  symbols: Record<HexString, string>;
};

function TransactionCard(props: Props) {
  const { timestamp, blockNumber, txHash, status, sourceNetwork } = props;

  const [isModalOpen, openModal, closeModal] = useModal();

  return (
    <>
      <Card className={styles.card}>
        <div className={styles.dateContainer}>
          <TransactionDate timestamp={timestamp} className={styles.date} />
          <BlockNumberLink blockNumber={blockNumber} sourceNetwork={sourceNetwork} />
        </div>

        <p className={styles.transactionHash}>
          <button type="button" onClick={openModal}>
            {getTruncatedText(txHash)}
          </button>

          <CopyButton value={txHash} />
        </p>

        <TransactionPair {...props} />
        <TransactionStatus status={status} />
      </Card>

      {isModalOpen && <TransactionModal close={closeModal} {...props} />}
    </>
  );
}

function TransactionCardSkeleton() {
  return (
    <Card className={styles.card}>
      <TransactionDate.Skeleton />

      <p className={styles.transactionHash}>
        <Skeleton>
          <span>0x000000000</span>
        </Skeleton>

        <Skeleton width="18px" height="18px" />
      </p>

      <TransactionPair.Skeleton />
      <TransactionStatus.Skeleton />
    </Card>
  );
}

TransactionCard.Skeleton = TransactionCardSkeleton;
export { TransactionCard };
