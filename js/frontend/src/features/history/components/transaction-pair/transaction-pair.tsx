import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { getPairHash } from '@workspace/common';

import { Address, FormattedBalance, Skeleton, TokenSVG } from '@/components';
import { Token } from '@/context';
import { NETWORK } from '@/features/swap/consts';

import ArrowSVG from '../../assets/arrow.svg?react';
import { Network, Transfer } from '../../types';

import styles from './transaction-pair.module.scss';

const INDEXED_NETWORK_TO_NETWORK = {
  [Network.Vara]: NETWORK.VARA,
  [Network.Ethereum]: NETWORK.ETH,
} as const;

type Props = Pick<
  Transfer,
  'sourceNetwork' | 'destNetwork' | 'source' | 'destination' | 'amount' | 'sender' | 'receiver'
> & {
  pairHashToToken: Record<string, Token>;
};

function TransactionPair(props: Props) {
  const { sourceNetwork, destNetwork, source, destination, amount, sender, receiver, pairHashToToken } = props;

  const sourceHex = source as HexString;
  const destinationHex = destination as HexString;

  const sourceToken = pairHashToToken[getPairHash(sourceHex, destinationHex)];
  const sourceSymbol = sourceToken?.displaySymbol ?? 'Unit';

  const destinationToken = pairHashToToken[getPairHash(destinationHex, sourceHex)];
  const destinationSymbol = destinationToken?.displaySymbol ?? 'Unit';

  const isVaraNetwork = sourceNetwork === Network.Vara;
  const formattedSenderAddress = isVaraNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isVaraNetwork ? receiver : getVaraAddress(receiver);

  return (
    <div className={styles.pair}>
      <div className={styles.tx}>
        <TokenSVG symbol={sourceSymbol} network={INDEXED_NETWORK_TO_NETWORK[sourceNetwork]} />

        <div>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={sourceToken?.decimals ?? 0}
            symbol={sourceSymbol}
            className={styles.amount}
          />

          <Address value={formattedSenderAddress} className={styles.address} />
        </div>
      </div>

      <ArrowSVG className={styles.arrow} />

      <div className={styles.tx}>
        <TokenSVG symbol={destinationSymbol} network={INDEXED_NETWORK_TO_NETWORK[destNetwork]} />

        <div>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={destinationToken?.decimals ?? 0}
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
        <TokenSVG.Skeleton />

        <div>
          <p className={styles.amount}>
            <Skeleton width="50%" />
          </p>

          <Skeleton>
            <span className={styles.address}>0x000000000000</span>
          </Skeleton>
        </div>
      </div>

      <Skeleton>
        <ArrowSVG className={styles.arrow} />
      </Skeleton>

      <div className={styles.tx}>
        <TokenSVG.Skeleton />

        <div>
          <p className={styles.amount}>
            <Skeleton width="50%" />
          </p>

          <Skeleton>
            <span className={styles.address}>0x000000000000</span>
          </Skeleton>
        </div>
      </div>
    </div>
  );
}

TransactionPair.Skeleton = TransactionPairSkeleton;

export { TransactionPair };
