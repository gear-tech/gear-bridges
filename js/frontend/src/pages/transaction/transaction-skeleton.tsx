import { Container, Card, Skeleton } from '@/components';
import { cx } from '@/utils';

import { Field } from './field';
import { SectionCard } from './section-card';
import styles from './transaction.module.scss';

function TransactionSkeleton() {
  return (
    <Container className={styles.container}>
      <header className={styles.header}>
        <div>
          <div className={styles.headingContainer}>
            <h1 className={styles.heading}>Transaction</h1>
            <Skeleton width="128px" />
          </div>

          <p className={styles.subheading}>Cross-chain swap transaction information</p>
        </div>
      </header>

      <div className={styles.cards}>
        <SectionCard heading="Overview" gridContent={false}>
          <Card className={styles.transaction}>
            <div className={styles.token}>
              <Skeleton width="48px" height="48px" borderRadius="50%" />

              <div>
                <Skeleton width="120px" className={styles.amount} />
                <Skeleton width="100px" className={styles.network} />
              </div>
            </div>

            <Skeleton className={cx(styles.arrow, styles.skeleton)} />

            <div className={styles.token}>
              <Skeleton width="48px" height="48px" borderRadius="50%" />

              <div>
                <Skeleton width="120px" className={styles.amount} />
                <Skeleton width="100px" className={styles.network} />
              </div>
            </div>
          </Card>
        </SectionCard>

        <SectionCard heading="Addresses">
          <Field label="From Address">
            <Skeleton width="25%" />
          </Field>

          <Field label="To Address">
            <Skeleton width="25%" />
          </Field>

          <Field label="Source Contract Address">
            <Skeleton width="25%" />
          </Field>

          <Field label="Destination Contract Address">
            <Skeleton width="25%" />
          </Field>
        </SectionCard>
      </div>
    </Container>
  );
}

export { TransactionSkeleton };
