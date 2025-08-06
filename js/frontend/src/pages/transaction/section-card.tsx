import { PropsWithChildren } from 'react';

import { Card } from '@/components';

import styles from './transaction.module.scss';

type Props = PropsWithChildren & {
  heading: string;
  gridContent?: boolean;
};

function SectionCard({ heading, children, gridContent = true }: Props) {
  return (
    <Card className={styles.section}>
      <h2 className={styles.heading}>{heading}</h2>
      <div className={gridContent ? styles.content : undefined}>{children}</div>
    </Card>
  );
}

export { SectionCard };
