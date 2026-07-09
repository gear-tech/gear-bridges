import SwapSVG from '../../assets/swap.svg?react';
import { useBridgeContext } from '../../context';
import { isEthToVaraUsdcBridgeDisabled } from '../../utils';

import styles from './swap-network-button.module.scss';

function SwapNetworkButton() {
  const { network, destinationToken } = useBridgeContext();
  const isSwitchDisabled = isEthToVaraUsdcBridgeDisabled(destinationToken);

  return (
    <button
      type="button"
      color="contrast"
      className={styles.button}
      onClick={network.switch}
      disabled={isSwitchDisabled}
      title={isSwitchDisabled ? 'USDC bridging from Ethereum is temporarily unavailable' : undefined}>
      <SwapSVG className={styles.icon} />
    </button>
  );
}

export { SwapNetworkButton };
