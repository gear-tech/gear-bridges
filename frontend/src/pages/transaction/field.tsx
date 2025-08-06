import { PropsWithChildren } from 'react';

import { Card } from '@/components';

import styles from './transaction.module.scss';

type Props = PropsWithChildren & {
  label: string;
};

function Field({ label, children }: Props) {
  return (
    <div className={styles.field}>
      <span className={styles.label}>{label}:</span>
      <Card className={styles.value}>{children}</Card>
    </div>
  );
}

export { Field };
