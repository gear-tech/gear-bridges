import { cx } from '@/utils';

import { STATUS_SVG } from '../../consts';
import { Teleport } from '../../types';

import styles from './transaction-card.module.scss';

function Status({ status }: Pick<Teleport, 'status'>) {
  const StatusSVG = STATUS_SVG[status];

  return (
    <div className={cx(styles.status, styles[status])}>
      <StatusSVG />
      {status.split(/(?=[A-Z])/).join(' ')}
    </div>
  );
}

export { Status };
