import { CSSProperties } from 'react';

import styles from './transaction-loading-bar.module.scss';

type Props = {
  status: 'mint' | 'approve' | 'transfer';
};

function TransactionLoadingBar({ status }: Props) {
  const getPercentage = () => {
    switch (status) {
      case 'mint':
        return 25;

      case 'approve':
        return 50;
      case 'transfer':
        return 75;

      default:
        return 0;
    }
  };

  const getText = () => {
    switch (status) {
      case 'mint':
        return 'Locking';

      case 'approve':
        return 'Approving';

      case 'transfer':
        return 'Transferring';

      default:
        return '';
    }
  };

  const style = { '--width': `${getPercentage()}%` } as CSSProperties;

  return (
    <div className={styles.container}>
      <p className={styles.text}>{getText()}</p>
      <div className={styles.bar} style={style} />
    </div>
  );
}

export { TransactionLoadingBar };
