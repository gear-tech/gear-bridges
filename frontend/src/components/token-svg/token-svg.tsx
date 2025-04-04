import { HexString } from '@gear-js/api';

import { NETWORK_INDEX } from '@/features/swap/consts';
import { getTokenSVG } from '@/utils';

import EthSVG from '../../assets/eth.svg?react';
import VaraSVG from '../../assets/vara.svg?react';
import { Skeleton } from '../layout';

import styles from './token-svg.module.scss';

const NETWORK_SVG = {
  [NETWORK_INDEX.VARA]: VaraSVG,
  [NETWORK_INDEX.ETH]: EthSVG,
} as const;

type Props = {
  address: HexString | undefined;
  networkIndex: number;
  sizes: [number, number];
};

function TokenSVG({ address, networkIndex, sizes }: Props) {
  const [size, smallSize] = sizes;
  const style = { width: `${size}px`, height: `${size}px` };
  const networkStyle = { width: `${smallSize}px`, height: `${smallSize}px`, marginLeft: `-${smallSize / 2}px` };

  const SVG = address ? getTokenSVG(address) : Skeleton;
  const NetworkSVG = NETWORK_SVG[networkIndex];

  return (
    <div className={styles.container} style={style}>
      <SVG className={styles.tokenSvg} />
      <NetworkSVG className={styles.networkSvg} style={networkStyle} />
    </div>
  );
}

export { TokenSVG };
