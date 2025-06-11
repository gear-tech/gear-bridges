import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';

import { Address, FormattedBalance, Skeleton, TokenSVG } from '@/components';
import { NETWORK_INDEX as DEFAULT_NETWORK_INDEX } from '@/features/swap/consts';

import ArrowSVG from '../../assets/arrow.svg?react';
import { Network, Transfer } from '../../types';

import styles from './transaction-pair.module.scss';

const NETWORK_INDEX = {
  [Network.Gear]: DEFAULT_NETWORK_INDEX.VARA,
  [Network.Ethereum]: DEFAULT_NETWORK_INDEX.ETH,
} as const;

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
  const sourceSymbol = symbols[sourceHex] ?? 'Unit';

  const destinationHex = destination as HexString;
  const destinationSymbol = symbols[destinationHex] ?? 'Unit';

  const isGearNetwork = sourceNetwork === Network.Gear;
  const formattedSenderAddress = isGearNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isGearNetwork ? receiver : getVaraAddress(receiver);

  return (
    <div className={styles.pair}>
      <div className={styles.tx}>
        <TokenSVG symbol={sourceSymbol} networkIndex={NETWORK_INDEX[sourceNetwork]} sizes={[32, 20]} />

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
        <TokenSVG symbol={destinationSymbol} networkIndex={NETWORK_INDEX[destNetwork]} sizes={[32, 20]} />

        <div>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={decimals[destinationHex] ?? 0}
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
        <TokenSVG.Skeleton sizes={[32, 20]} />

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
        <TokenSVG.Skeleton sizes={[32, 20]} />

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
