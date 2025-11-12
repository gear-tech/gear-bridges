import { Button, Modal } from '@gear-js/vara-ui';
import { useChainId } from 'wagmi';

import { cx } from '@/utils';

import { useNetworkType } from './context';
import styles from './eth-network-mismatch-modal.module.scss';

type Props = {
  onButtonClick: () => void;
};

function EthNetworkMismatchModal({ onButtonClick }: Props) {
  const { NETWORK_PRESET, isMainnet } = useNetworkType();
  const wagmiChainId = useChainId();

  if (wagmiChainId === NETWORK_PRESET.ETH_CHAIN_ID) return;

  return (
    <Modal heading="Network Mismatch" className={cx('unsupportedNetworkModal', styles.modal)} close={() => {}}>
      <div className={styles.text}>
        <p>You are connected to a different Ethereum network.</p>

        <p>
          Switch to {NETWORK_PRESET.ETH_NETWORK.name} {isMainnet ? 'Mainnet' : 'Testnet'} to match the selected network
          type.
        </p>
      </div>

      <Button text="Switch" size="x-small" onClick={onButtonClick} block />
    </Modal>
  );
}

export { EthNetworkMismatchModal };
