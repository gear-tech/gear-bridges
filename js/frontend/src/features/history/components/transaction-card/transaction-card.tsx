import { HexString } from '@gear-js/api';
import { generatePath, Link } from 'react-router-dom';

import { Card, CopyButton, Skeleton, Tooltip } from '@/components';
import { ROUTE } from '@/consts';
import { Token } from '@/context';
import { cx, getTruncatedText } from '@/utils';

import { Transfer } from '../../types';
import { BlockNumberLink } from '../block-number-link';
import { TransactionDate } from '../transaction-date';
import { TransactionPair } from '../transaction-pair';
import { TransactionStatus } from '../transaction-status';

import styles from './transaction-card.module.scss';

type Props = Pick<
  Transfer,
  | 'id'
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
  getHistoryToken: (sourceAddress: HexString, destinationAddress: HexString) => Token;
};

function TransactionCard(props: Props) {
  const { id, timestamp, blockNumber, txHash, status, sourceNetwork } = props;

  return (
    <Card className={styles.card}>
      <Link to={generatePath(ROUTE.TRANSACTION, { id })} className={styles.info}>
        <TransactionDate timestamp={timestamp} className={styles.date} />

        <p className={styles.hash}>{getTruncatedText(txHash)}</p>

        <TransactionPair {...props} />
        <TransactionStatus status={status} />
      </Link>

      <div className={styles.actions}>
        <Tooltip value="Copy Transaction Hash">
          <CopyButton value={txHash} />
        </Tooltip>

        <BlockNumberLink blockNumber={blockNumber} sourceNetwork={sourceNetwork} />
      </div>
    </Card>
  );
}

function TransactionCardSkeleton() {
  return (
    <Card className={cx(styles.card, styles.skeleton)}>
      <div className={styles.info}>
        <TransactionDate.Skeleton />

        <Skeleton>
          <span className={styles.hash}>0x000000000</span>
        </Skeleton>

        <TransactionPair.Skeleton />
        <TransactionStatus.Skeleton />
      </div>

      <div className={styles.actions}>
        <Skeleton width="16px" height="16px" borderRadius="50%" />
        <Skeleton width="16px" height="16px" borderRadius="50%" />
      </div>
    </Card>
  );
}

TransactionCard.Skeleton = TransactionCardSkeleton;
export { TransactionCard };
