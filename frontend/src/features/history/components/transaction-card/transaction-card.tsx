import { HexString } from '@gear-js/api';

import { Card, CopyButton, Skeleton, TruncatedText } from '@/components';

import { Transfer } from '../../types';
import { TransactionDate } from '../transaction-date';
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
  | 'blockNumber'
  | 'sender'
  | 'receiver'
> & {
  decimals: Record<HexString, number>;
  symbols: Record<HexString, string>;
};

function TransactionCard({ status, timestamp, blockNumber, ...props }: Props) {
  return (
    <Card className={styles.wideCard}>
      <TransactionDate timestamp={timestamp} />

      <p className={styles.blockNumber}>
        <TruncatedText value={blockNumber} />
        <CopyButton value={blockNumber} />
      </p>

      <TransactionPair {...props} />
      <TransactionStatus status={status} />
    </Card>
  );
}

function TransactionCardCompact({ status, timestamp, ...props }: Props) {
  return (
    <Card className={styles.compactCard}>
      <TransactionPair {...props} isCompact />

      <div>
        <TransactionStatus status={status} />
        <TransactionDate timestamp={timestamp} isCompact />
      </div>
    </Card>
  );
}

function TransactionCardSkeleton() {
  return (
    // TODO: make detailed
    <Skeleton>
      <Card className={styles.compactCard}>
        <TransactionPair.Skeleton />

        <div>
          <TransactionStatus.Skeleton />
          <TransactionDate.Skeleton />
        </div>
      </Card>
    </Skeleton>
  );
}

TransactionCard.Skeleton = TransactionCardSkeleton;
TransactionCard.Compact = TransactionCardCompact;

export { TransactionCard };
