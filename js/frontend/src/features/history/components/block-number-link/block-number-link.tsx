import { LinkButton, Tooltip } from '@/components';
import { ETH_EXPLORER_URL, VARA_ARCHIVE_NODE_ADDRESS, VARA_EXPLORER_URL } from '@/consts';

import CubeSVG from '../../assets/cube.svg?react';
import { Network, Transfer } from '../../types';

import styles from './block-number-link.module.scss';

const EXPLORER_URL = {
  [Network.Vara]: VARA_EXPLORER_URL
    ? `${VARA_EXPLORER_URL}/block`
    : `https://polkadot.js.org/apps/?rpc=${VARA_ARCHIVE_NODE_ADDRESS}#/explorer/query`,

  [Network.Ethereum]: `${ETH_EXPLORER_URL}/block`,
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
