import { LinkButton, Tooltip } from '@/components';

import CubeSVG from '../../assets/cube.svg?react';
import { EXPLORER_URL } from '../../consts';
import { Transfer } from '../../types';

import styles from './block-number-link.module.scss';

const FORMATTER = new Intl.NumberFormat();

type Props = Pick<Transfer, 'blockNumber' | 'sourceNetwork'>;

function BlockNumberLink({ blockNumber, sourceNetwork }: Props) {
  const formattedBlockNumber = FORMATTER.format(BigInt(blockNumber));
  const explorerUrl = EXPLORER_URL[sourceNetwork];

  return (
    <Tooltip value={`Block #${formattedBlockNumber}`}>
      <LinkButton
        type="external"
        to={`${explorerUrl}/block/${blockNumber}`}
        icon={CubeSVG}
        color="transparent"
        size="x-small"
        className={styles.link}
      />
    </Tooltip>
  );
}

export { BlockNumberLink };
