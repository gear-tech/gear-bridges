import { formatUnits } from 'viem';

import ClockSVG from '@/assets/clock.svg?react';
import { TOKEN_ID, TokenPrice } from '@/features/token-price';
import { useVaraSymbol } from '@/hooks';
import { cx } from '@/utils';

import { Skeleton } from '../layout';

import styles from './fee-and-time-footer.module.scss';
import GasSVG from './gas.svg?react';

type Props = {
  feeValue: bigint | undefined;
  decimals: number | undefined;
  isVaraNetwork: boolean;
  isLoading?: boolean;
  className?: string;
};

function FeeAndTimeFooter({ feeValue, decimals, isVaraNetwork, isLoading, className }: Props) {
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
          {feeValue && decimals && symbol ? (
            <>
              {formatUnits(feeValue, decimals)} {symbol}
              <TokenPrice id={tokenId} amount={formatUnits(feeValue, decimals)} fraction={4} />
            </>
          ) : (
            <>
              <Skeleton width="3.5rem" disabled={!isLoading} />
              <Skeleton width="3.5rem" disabled={!isLoading} />
            </>
          )}
        </span>
      </p>

      <p className={styles.prop}>
        <span className={styles.key}>
          <ClockSVG /> Bridge Time:
        </span>

        <span className={styles.value}>~20 mins</span>
      </p>
    </footer>
  );
}

export { FeeAndTimeFooter };
