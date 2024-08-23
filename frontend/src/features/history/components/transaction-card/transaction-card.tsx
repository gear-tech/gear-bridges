import { Card, CopyButton, Skeleton, TruncatedText } from '@/components';

import ArrowSVG from '../../assets/arrow.svg?react';
import { Teleport } from '../../types';

import { Sources } from './sources';
import { Status } from './status';
import { Time } from './time';
import styles from './transaction-card.module.scss';

type Props = Pick<Teleport, 'amount' | 'from' | 'to' | 'status' | 'timestamp' | 'direction' | 'blockhash' | 'pair'> & {
  isCompact?: boolean;
};

function TransactionCard({ amount, from, to, status, timestamp, direction, blockhash, pair, isCompact }: Props) {
  if (isCompact)
    return (
      <Card className={styles.compactCard}>
        <Sources direction={direction} from={from} to={to} amount={amount} pair={pair} isCompact />

        <div>
          <Status status={status} />
          <Time timestamp={timestamp} isCompact />
        </div>
      </Card>
    );

  return (
    <Card className={styles.wideCard}>
      <Time timestamp={timestamp} />

      <p className={styles.blockhash}>
        <TruncatedText value={blockhash} />
        <CopyButton value={blockhash} />
      </p>

      <Sources direction={direction} from={from} to={to} pair={pair} amount={amount} />
      <Status status={status} />
    </Card>
  );
}

function TransactionCardSkeleton() {
  return (
    // TODO: make detailed
    <Skeleton>
      <Card className={styles.compactCard}>
        <div className={styles.sources}>
          <div className={styles.source}>
            <div className={styles.icons}>
              <ArrowSVG />
              <ArrowSVG />
            </div>

            <div>
              <p className={styles.amount}>0.0000 Unit</p>
              <TruncatedText value="0x00" className={styles.address} />
            </div>
          </div>

          <ArrowSVG />

          <div className={styles.source}>
            <div className={styles.icons}>
              <ArrowSVG />
              <ArrowSVG />
            </div>

            <div>
              <p className={styles.amount}>0.0000 Unit</p>
              <TruncatedText value="0x00" className={styles.address} />
            </div>
          </div>
        </div>

        <div>
          <div className={styles.status}>Status</div>

          <p className={styles.date}>
            <ArrowSVG /> 01.01.1970 00:00:00
          </p>
        </div>
      </Card>
    </Skeleton>
  );
}

TransactionCard.Skeleton = TransactionCardSkeleton;

export { TransactionCard };
