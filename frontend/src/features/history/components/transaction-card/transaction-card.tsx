import { HexString } from '@gear-js/api';

import { Card, CopyButton, Skeleton, TruncatedText } from '@/components';
import { useModal } from '@/hooks';
import { cx } from '@/utils';

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
      <Card className={cx(styles.wideCard, styles.button)}>
        <TransactionDate timestamp={timestamp} />

        <p className={styles.transactionHash}>
          <button type="button" onClick={openModal}>
            <TruncatedText value={txHash} />
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
    <Card className={styles.wideCard}>
      <TransactionDate.Skeleton />

      <p className={styles.transactionHash}>
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

export { TransactionCard };
