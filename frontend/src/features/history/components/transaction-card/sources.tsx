import { getVaraAddress } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import { TruncatedText, Skeleton } from '@/components';
import { NETWORK_NAME, SPEC } from '@/consts';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { DIRECTION_NETWORK_NAME, DIRECTION_NETWORK_SVG } from '../../consts';
import { Direction, Teleport } from '../../types';

import styles from './transaction-card.module.scss';

type Props = Pick<Teleport, 'direction' | 'from' | 'to' | 'amount' | 'pair'> & {
  isCompact?: boolean;
};

function Sources({ direction, from, to, amount, isCompact, pair }: Props) {
  const networkName = DIRECTION_NETWORK_NAME[direction];
  const isVaraNetwork = networkName === NETWORK_NAME.VARA;

  const bridge = SPEC[pair as keyof typeof SPEC]; // assertion cuz usdt bridge is not yet implemented
  const fromBridge = bridge[networkName];
  const toBridge = bridge[isVaraNetwork ? NETWORK_NAME.ETH : NETWORK_NAME.VARA];

  const { symbol: fromSymbol, SVG: FromCoinSVG, decimals } = fromBridge;
  const { symbol: toSymbol, SVG: ToCoinSVG } = toBridge;

  const FromNetworkSVG = DIRECTION_NETWORK_SVG[direction];
  const ToNetworkSVG = DIRECTION_NETWORK_SVG[isVaraNetwork ? Direction.EthToVara : Direction.VaraToEth];

  const formattedFromAddress = isVaraNetwork ? getVaraAddress(from) : `0x${from}`;
  const formattedToAddress = isVaraNetwork ? `0x${to}` : getVaraAddress(to);

  const formattedAmount = formatUnits(BigInt(amount), decimals);

  return (
    <div className={cx(styles.sources, isCompact && styles.compact)}>
      <div className={styles.source}>
        <div className={styles.icons}>
          <FromNetworkSVG />
          <FromCoinSVG />
        </div>

        <div>
          {formattedAmount ? (
            <TruncatedText value={`${formattedAmount} ${fromSymbol}`} className={styles.amount} />
          ) : (
            <Skeleton />
          )}

          <TruncatedText value={formattedFromAddress} className={styles.address} />
        </div>
      </div>

      <ArrowSVG />

      <div className={styles.source}>
        <div className={styles.icons}>
          <ToNetworkSVG />
          <ToCoinSVG />
        </div>

        <div>
          {formattedAmount ? (
            <TruncatedText value={`${formattedAmount} ${toSymbol}`} className={styles.amount} />
          ) : (
            <Skeleton />
          )}

          <TruncatedText value={formattedToAddress} className={styles.address} />
        </div>
      </div>
    </div>
  );
}

export { Sources };
