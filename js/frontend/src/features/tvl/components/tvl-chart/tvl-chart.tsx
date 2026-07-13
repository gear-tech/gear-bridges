import { useState } from 'react';

import { Skeleton } from '@/components';

import { CHART_URL } from '../../api';

import styles from './tvl-chart.module.scss';

function TvlChart() {
  const [isLoaded, setIsLoaded] = useState(false);

  return (
    <div className={styles.chart}>
      {!isLoaded && <Skeleton className={styles.skeleton} borderRadius="8px" />}

      <iframe
        className={styles.iframe}
        data-loaded={isLoaded}
        src={CHART_URL}
        title="DefiLlama TVL Chart"
        loading="lazy"
        onLoad={() => setIsLoaded(true)}
      />
    </div>
  );
}

export { TvlChart };
