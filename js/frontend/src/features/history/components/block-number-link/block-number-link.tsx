import { LinkButton, Tooltip } from '@/components';
import { useNetworkType } from '@/context/network-type';

import CubeSVG from '../../assets/cube.svg?react';
import { Network, Transfer } from '../../types';

import styles from './block-number-link.module.scss';

function useExplorerUrl(network: Network) {
  const { NETWORK_PRESET } = useNetworkType();

  const networkToExplorerUrl = {
    [Network.Vara]: NETWORK_PRESET.EXPLORER_URL.VARA
      ? `${NETWORK_PRESET.EXPLORER_URL.VARA}/block`
      : `https://polkadot.js.org/apps/?rpc=${NETWORK_PRESET.ARCHIVE_NODE_ADDRESS}#/explorer/query`,

    [Network.Ethereum]: `${NETWORK_PRESET.EXPLORER_URL.ETH}/block`,
  };

  return networkToExplorerUrl[network];
}

const FORMATTER = new Intl.NumberFormat();

type Props = Pick<Transfer, 'blockNumber' | 'sourceNetwork'>;

function BlockNumberLink({ blockNumber, sourceNetwork }: Props) {
  const formattedBlockNumber = FORMATTER.format(BigInt(blockNumber));
  const explorerUrl = useExplorerUrl(sourceNetwork);

  return (
    <Tooltip value={`Block #${formattedBlockNumber}`}>
      <LinkButton
        type="external"
        to={`${explorerUrl}/${blockNumber}`}
        icon={CubeSVG}
        color="transparent"
        size="x-small"
        className={styles.link}
        onClick={(e) => e.stopPropagation()}
      />
    </Tooltip>
  );
}

export { BlockNumberLink };
