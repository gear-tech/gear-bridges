import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { Address, FormattedBalance, Skeleton } from '@/components';
import { TOKEN_SVG } from '@/consts';

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
};

function TransactionPair(props: Props) {
  const { sourceNetwork, destNetwork, source, destination, amount, sender, receiver, symbols, decimals } = props;

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

  return (
    <div className={styles.pair}>
      <div className={styles.tx}>
        <div className={styles.icons}>
          <SourceNetworkSVG />
          <SourceTokenSVG />
        </div>

        <div>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={decimals[sourceHex] ?? 0}
            symbol={sourceSymbol}
            className={styles.amount}
          />

          <Address value={formattedSenderAddress} className={styles.address} />
        </div>
      </div>

      <ArrowSVG />

      <div className={styles.tx}>
        <div className={styles.icons}>
          <DestinationNetworkSVG />
          <DestinationTokenSVG />
        </div>

        <div>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={decimals[sourceHex] ?? 0}
            symbol={destinationSymbol}
            className={styles.amount}
          />

          <Address value={formattedReceiverAddress} className={styles.address} />
        </div>
      </div>
    </div>
  );
}

function TransactionPairSkeleton() {
  return (
    <div className={styles.pair}>
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
