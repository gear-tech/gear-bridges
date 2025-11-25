import { HexString } from '@gear-js/api';
import { generatePath, Link } from 'react-router-dom';

import { Card, CopyButton, Skeleton } from '@/components';
import { ROUTE } from '@/consts';
import { Token } from '@/context';
import { getTruncatedText } from '@/utils';

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
    <Card as={Link} to={generatePath(ROUTE.TRANSACTION, { id })} className={styles.card}>
      <div className={styles.dateContainer}>
        <TransactionDate timestamp={timestamp} className={styles.date} />
        <BlockNumberLink blockNumber={blockNumber} sourceNetwork={sourceNetwork} />
      </div>

      <p className={styles.transactionHash}>
        {getTruncatedText(txHash)}
        <CopyButton value={txHash} stopPropagation />
      </p>

      <TransactionPair {...props} />
      <TransactionStatus status={status} />
    </Card>
  );
}

function TransactionCardSkeleton() {
  return (
    <Card className={styles.card}>
      <TransactionDate.Skeleton />

      <p className={styles.transactionHash}>
        <Skeleton>
          <span className={styles.link}>0x000000000</span>
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
