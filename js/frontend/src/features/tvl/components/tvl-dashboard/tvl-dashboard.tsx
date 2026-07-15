import { useNetworkType } from '@/context/network-type';

import { PROTOCOL_URL } from '../../api';
import { useTvl } from '../../hooks/use-tvl';
import { TvlChart } from '../tvl-chart';
import { TvlFallback } from '../tvl-fallback';
import { TvlSummary } from '../tvl-summary';

import styles from './tvl-dashboard.module.scss';

function TvlDashboard() {
  const { data, isLoading, isError } = useTvl();
  const { NETWORK_PRESET } = useNetworkType();

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
        Data provided by DeFi Llama ↗
      </a>

      <br />

      <a
        className={styles.attribution}
        href={`${NETWORK_PRESET.EXPLORER_URL.ETH}/address/${NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS}`}
        target="_blank"
        rel="noreferrer">
        ERC20Manager on Etherscan ↗
      </a>
    </div>
  );
}

export { TvlDashboard };
