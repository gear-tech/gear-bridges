import { LinkButton } from '@/components';

import { PROTOCOL_URL } from '../../api';

import styles from './tvl-fallback.module.scss';

function TvlFallback() {
  return (
    <div className={styles.fallback}>
      <p className={styles.message}>Unable to load TVL data. Please try again later.</p>

      <LinkButton to={PROTOCOL_URL} type="external" text="View on DeFiLlama" />
    </div>
  );
}

export { TvlFallback };
