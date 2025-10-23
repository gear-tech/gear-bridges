import { LinkButton, Tooltip } from '@/components';
import { ETH_EXPLORER_URL, NETWORK_TYPE, networkType, VARA_ARCHIVE_NODE_ADDRESS } from '@/consts';

import CubeSVG from '../../assets/cube.svg?react';
import { Network, Transfer } from '../../types';

import styles from './block-number-link.module.scss';

const NETWORK_TYPE_TO_VARA_EXPLORER_URL = {
  [NETWORK_TYPE.MAINNET]: `https://vara.subscan.io/block`,
  [NETWORK_TYPE.TESTNET]: `https://polkadot.js.org/apps/?rpc=${VARA_ARCHIVE_NODE_ADDRESS}#/explorer/query/block`,
} as const;

const VARA_EXPLORER_URL = NETWORK_TYPE_TO_VARA_EXPLORER_URL[networkType];

const EXPLORER_URL = {
  [Network.Vara]: VARA_EXPLORER_URL,
  [Network.Ethereum]: ETH_EXPLORER_URL,
} as const;

const FORMATTER = new Intl.NumberFormat();

type Props = Pick<Transfer, 'blockNumber' | 'sourceNetwork'>;

function BlockNumberLink({ blockNumber, sourceNetwork }: Props) {
  const formattedBlockNumber = FORMATTER.format(BigInt(blockNumber));
  const explorerUrl = EXPLORER_URL[sourceNetwork];

  return (
    <Tooltip value={`Block #${formattedBlockNumber}`}>
      <LinkButton
        type="external"
        to={`${explorerUrl}/${blockNumber}`}
        icon={CubeSVG}
        color="transparent"
        size="x-small"
        className={styles.link}
      />
    </Tooltip>
  );
}

export { BlockNumberLink };
