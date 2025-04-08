import { HexString } from '@gear-js/api';

import { Address, Card, CopyButton, Skeleton } from '@/components';
import { useModal } from '@/hooks';

import { Transfer } from '../../types';
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
> & {
  decimals: Record<HexString, number>;
  symbols: Record<HexString, string>;
};

function TransactionCard(props: Props) {
  const { timestamp, txHash, status } = props;

  const [isModalOpen, openModal, closeModal] = useModal();

  return (
    <>
      <Card className={styles.card}>
        <TransactionDate timestamp={timestamp} className={styles.date} />

        <p className={styles.transactionHash}>
          <button type="button" onClick={openModal}>
            <Address value={txHash} />
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
