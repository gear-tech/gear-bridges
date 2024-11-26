import { HexString } from '@gear-js/api';

import { Card, CopyButton, Skeleton } from '@/components';

import { Network, Transfer } from '../../types';
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
  const explorerUrl =
    props.sourceNetwork === Network.Gear ? 'https://vara.subscan.io/block' : 'https://etherscan.io/block';

  return (
    <Card className={styles.wideCard}>
      <TransactionDate timestamp={timestamp} />

      <p className={styles.blockNumber}>
        <a href={`${explorerUrl}/${blockNumber}`} target="_blank" rel="noreferrer">
          {blockNumber}
        </a>

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

function TransactionCardSkeleton({ isCompact }: { isCompact?: boolean }) {
  if (isCompact)
    return (
      <Card className={styles.compactCard}>
        <TransactionPair.Skeleton isCompact />

        <div>
          <TransactionStatus.Skeleton />
          <TransactionDate.Skeleton isCompact />
        </div>
      </Card>
    );

  return (
    <Card className={styles.wideCard}>
      <TransactionDate.Skeleton />

      <p className={styles.blockNumber}>
        <Skeleton>
          <span>0x000000000</span>
        </Skeleton>

        <Skeleton>
          <CopyButton value="" />
        </Skeleton>
      </p>

      <TransactionPair.Skeleton />
      <TransactionStatus.Skeleton />
    </Card>
  );
}

TransactionCard.Skeleton = TransactionCardSkeleton;
TransactionCard.Compact = TransactionCardCompact;

export { TransactionCard };
