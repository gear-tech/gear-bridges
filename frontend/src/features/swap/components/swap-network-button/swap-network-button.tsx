import SwapSVG from '../../assets/swap.svg?react';
import { useBridgeContext } from '../../context';

import styles from './swap-network-button.module.scss';

function SwapNetworkButton() {
  const { network } = useBridgeContext();

  return (
    <button type="button" color="contrast" className={styles.button} onClick={network.switch}>
      <SwapSVG className={styles.icon} />
    </button>
  );
}

export { SwapNetworkButton };
