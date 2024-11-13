import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TruncatedText } from '@/components';
import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import TokenPlaceholderSVG from '../../assets/token-placeholder.svg?react';
import UsdcSVG from '../../assets/usdc.svg?react';
import VaraUsdcSVG from '../../assets/vara-usdc.svg?react';
import WrappedEthSVG from '../../assets/wrapped-eth.svg?react';
import WrappedVaraSVG from '../../assets/wrapped-vara.svg?react';
import { Network, Transfer } from '../../types';

import styles from './transaction-pair.module.scss';

const NETWORK_SVG = {
  [Network.Gear]: VaraSVG,
  [Network.Ethereum]: EthSVG,
} as const;

const TOKEN_SVG: Record<HexString, SVGComponent> = {
  '0x00': VaraSVG,
  '0x01': EthSVG,
  '0x02': WrappedVaraSVG,
  '0x03': WrappedEthSVG,
  '0x05': VaraUsdcSVG,
  '0x04': UsdcSVG,
};

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

function TransactionPairSkeleton() {
  return (
    <div className={styles.pair}>
      <div className={styles.tx}>
        <div className={styles.icons}>
          <ArrowSVG />
          <ArrowSVG />
        </div>

        <div>
          <p className={styles.amount}>0.0000 Unit</p>
          <TruncatedText value="0x00" className={styles.address} />
        </div>
      </div>

      <ArrowSVG />

      <div className={styles.tx}>
        <div className={styles.icons}>
          <ArrowSVG />
          <ArrowSVG />
        </div>

        <div>
          <p className={styles.amount}>0.0000 Unit</p>
          <TruncatedText value="0x00" className={styles.address} />
        </div>
      </div>
    </div>
  );
}

TransactionPair.Skeleton = TransactionPairSkeleton;

export { TransactionPair };
