import { PROTOCOL_URL } from '../../api';
import { useTvl } from '../../hooks/use-tvl';
import { TvlChart } from '../tvl-chart';
import { TvlFallback } from '../tvl-fallback';
import { TvlSummary } from '../tvl-summary';

import styles from './tvl-dashboard.module.scss';

function TvlDashboard() {
  const { data, isLoading, isError } = useTvl();

  return (
    <div className={styles.card}>
      {isError ? (
        <TvlFallback />
      ) : (
        <>
          <TvlSummary value={data} isLoading={isLoading} />
          <TvlChart />
        </>
      )}

      <a className={styles.attribution} href={PROTOCOL_URL} target="_blank" rel="noreferrer">
        Data provided by DeFiLlama ↗
      </a>
    </div>
  );
}

export { TvlDashboard };
