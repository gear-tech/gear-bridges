import { LinkButton, Tooltip } from '@/components';

import CubeSVG from '../../assets/cube.svg?react';
import { EXPLORER_URL } from '../../consts';
import { Network, Transfer } from '../../types';

import styles from './block-number-link.module.scss';

const FORMATTER = new Intl.NumberFormat();

type Props = Pick<Transfer, 'blockNumber' | 'sourceNetwork'>;

function BlockNumberLink({ blockNumber, sourceNetwork }: Props) {
  const formattedBlockNumber = FORMATTER.format(BigInt(blockNumber));
  const explorerUrl = EXPLORER_URL[sourceNetwork];
  const urlPath = sourceNetwork === Network.Vara ? '' : '/block';

  return (
    <Tooltip value={`Block #${formattedBlockNumber}`}>
      <LinkButton
        type="external"
        to={`${explorerUrl}${urlPath}/${blockNumber}`}
        icon={CubeSVG}
        color="transparent"
        size="x-small"
        className={styles.link}
      />
    </Tooltip>
  );
}

export { BlockNumberLink };
