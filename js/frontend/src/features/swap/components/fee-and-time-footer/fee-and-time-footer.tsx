import { formatUnits } from 'viem';

import ClockSVG from '@/assets/clock.svg?react';
import { Skeleton } from '@/components';
import { TOKEN_ID, TokenPrice } from '@/features/token-price';
import { useVaraSymbol } from '@/hooks';
import { cx } from '@/utils';

import GasSVG from '../../assets/gas.svg?react';

import styles from './fee-and-time-footer.module.scss';

type Props = {
  feeValue: bigint | undefined;
  time: string;
  isVaraNetwork: boolean;
  isLoading?: boolean;
  className?: string;
};

function FeeAndTimeFooter({ feeValue, time, isVaraNetwork, isLoading, className }: Props) {
  const varaSymbol = useVaraSymbol();

  const tokenId = isVaraNetwork ? TOKEN_ID.VARA : TOKEN_ID.ETH;
  const symbol = isVaraNetwork ? varaSymbol : 'ETH';
  const decimals = isVaraNetwork ? 12 : 18;

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

        <span className={styles.value}>~{time}</span>
      </p>
    </footer>
  );
}

export { FeeAndTimeFooter };
