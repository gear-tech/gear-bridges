import ClockSVG from '@/assets/clock.svg?react';
import { TOKEN_ID, TokenPrice } from '@/features/token-price';
import { useVaraSymbol } from '@/hooks';
import { cx } from '@/utils';

import { Skeleton } from '../layout';

import styles from './fee-and-time-footer.module.scss';
import GasSVG from './gas.svg?react';

type Props = {
  fee: string | undefined;
  isVaraNetwork: boolean;
  className?: string;
};

function FeeAndTimeFooter({ fee, isVaraNetwork, className }: Props) {
  const varaSymbol = useVaraSymbol();

  const tokenId = isVaraNetwork ? TOKEN_ID.VARA : TOKEN_ID.ETH;
  const symbol = isVaraNetwork ? varaSymbol : 'ETH';

  return (
    <footer className={cx(styles.footer, className)}>
      <p className={styles.prop}>
        <span className={styles.key}>
          <GasSVG /> Expected Fee:
        </span>

        <span className={styles.value}>
          {fee && symbol ? `${fee} ${symbol}` : <Skeleton width="3.5rem" />}
          <TokenPrice id={tokenId} amount={fee} />
        </span>
      </p>

      <p className={styles.prop}>
        <span className={styles.key}>
          <ClockSVG /> Bridge Time:
        </span>

        <span className={styles.value}>~30 mins</span>
      </p>
    </footer>
  );
}

export { FeeAndTimeFooter };
