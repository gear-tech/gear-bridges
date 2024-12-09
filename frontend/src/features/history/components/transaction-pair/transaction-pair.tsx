import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { Skeleton, TruncatedText } from '@/components';
import { TOKEN_SVG } from '@/consts';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK_SVG } from '../../consts';
import { Network, Transfer } from '../../types';

import styles from './transaction-pair.module.scss';

type Props = Pick<
  Transfer,
  'sourceNetwork' | 'destNetwork' | 'source' | 'destination' | 'amount' | 'sender' | 'receiver'
> & {
  symbols: Record<HexString, string>;
  decimals: Record<HexString, number>;
  isCompact?: boolean;
};

function TransactionPair(props: Props) {
  const { sourceNetwork, destNetwork, source, destination, amount, sender, receiver, symbols, decimals, isCompact } =
    props;

  const sourceHex = source as HexString;
  const SourceNetworkSVG = NETWORK_SVG[sourceNetwork];
  const SourceTokenSVG = TOKEN_SVG[sourceHex] ?? TokenPlaceholderSVG;
  const sourceSymbol = symbols[sourceHex] ?? 'Unit';

  const destinationHex = destination as HexString;
  const DestinationNetworkSVG = NETWORK_SVG[destNetwork];
  const DestinationTokenSVG = TOKEN_SVG[destinationHex] ?? TokenPlaceholderSVG;
  const destinationSymbol = symbols[destinationHex] ?? 'Unit';

  const isGearNetwork = sourceNetwork === Network.Gear;
  const formattedSenderAddress = isGearNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isGearNetwork ? receiver : getVaraAddress(receiver);

  const formattedAmount = formatUnits(BigInt(amount), decimals[sourceHex] ?? 0);

  return (
    <div className={cx(styles.pair, isCompact && styles.compact)}>
      <div className={styles.tx}>
        <div className={styles.icons}>
          <SourceNetworkSVG />
          <SourceTokenSVG />
        </div>

        <div>
          <TruncatedText value={`${formattedAmount} ${sourceSymbol}`} className={styles.amount} />
          <TruncatedText value={formattedSenderAddress} className={styles.address} />
        </div>
      </div>

      <ArrowSVG />

      <div className={styles.tx}>
        <div className={styles.icons}>
          <DestinationNetworkSVG />
          <DestinationTokenSVG />
        </div>

        <div>
          <TruncatedText value={`${formattedAmount} ${destinationSymbol}`} className={styles.amount} />
          <TruncatedText value={formattedReceiverAddress} className={styles.address} />
        </div>
      </div>
    </div>
  );
}

function TransactionPairSkeleton({ isCompact }: Pick<Props, 'isCompact'>) {
  return (
    <div className={cx(styles.pair, isCompact && styles.compact)}>
      <div className={styles.tx}>
        <div className={styles.icons}>
          <Skeleton>
            <VaraSVG />
          </Skeleton>

          <Skeleton>
            <VaraSVG />
          </Skeleton>
        </div>

        <div>
          <p className={styles.amount}>
            <Skeleton width="50%" />
          </p>

          <Skeleton>
            <span>0x000000000000</span>
          </Skeleton>
        </div>
      </div>

      <Skeleton>
        <ArrowSVG />
      </Skeleton>

      <div className={styles.tx}>
        <div className={styles.icons}>
          <Skeleton>
            <VaraSVG />
          </Skeleton>

          <Skeleton>
            <VaraSVG />
          </Skeleton>
        </div>

        <div>
          <p className={styles.amount}>
            <Skeleton width="50%" />
          </p>

          <Skeleton>
            <span>0x000000000000</span>
          </Skeleton>
        </div>
      </div>
    </div>
  );
}

TransactionPair.Skeleton = TransactionPairSkeleton;

export { TransactionPair };
