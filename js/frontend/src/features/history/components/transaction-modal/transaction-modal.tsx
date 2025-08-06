import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { JSX } from 'react';

import { Address, FeeAndTimeFooter, FormattedBalance } from '@/components';
import { useTokens } from '@/context';
import { getAddressToTokenKey } from '@/context/tokens';
import { cx, isUndefined } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK_SVG } from '../../consts';
import { Network, Transfer } from '../../types';

import styles from './transaction-modal.module.scss';

type Props = Pick<
  Transfer,
  'amount' | 'destination' | 'source' | 'sourceNetwork' | 'destNetwork' | 'sender' | 'receiver'
> & {
  estimatedFees: bigint;
  close: () => void;
  renderProgressBar: () => JSX.Element;
};

function TransactionModal({
  sourceNetwork,
  destNetwork,
  amount,
  source,
  destination,
  sender,
  receiver,
  estimatedFees,
  renderProgressBar,
  close,
}: Props) {
  const { addressToToken } = useTokens();
  const isVaraNetwork = sourceNetwork === Network.Vara;

  const SourceNetworkSVG = NETWORK_SVG[sourceNetwork];
  const DestinationNetworkSVG = NETWORK_SVG[destNetwork];

  const sourceToken = addressToToken?.[getAddressToTokenKey(source as HexString, destination as HexString)];
  const destinationToken = addressToToken?.[getAddressToTokenKey(destination as HexString, source as HexString)];

  const formattedSenderAddress = isVaraNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isVaraNetwork ? receiver : getVaraAddress(receiver);

  return (
    <Modal heading="Transaction Details" close={close} maxWidth="large">
      <div className={cx(styles.pairs, styles.loading)}>
        <span className={styles.tx}>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={sourceToken?.decimals ?? 0}
            symbol={sourceToken?.displaySymbol || 'Unit'}
            className={styles.amount}
          />

          <span className={styles.label}>on</span>

          <span className={styles.network}>
            <SourceNetworkSVG />
            {isVaraNetwork ? 'Vara' : sourceNetwork}
          </span>
        </span>

        <ArrowSVG className={styles.arrowSvg} />

        <span className={styles.tx}>
          <FormattedBalance
            value={BigInt(amount)}
            decimals={destinationToken?.decimals ?? 0}
            symbol={destinationToken?.displaySymbol || 'Unit'}
            className={styles.amount}
          />

          <span className={styles.label}>on</span>

          <span className={styles.network}>
            <DestinationNetworkSVG className={styles.networkSvg} />
            {isVaraNetwork ? destNetwork : 'Vara'}
          </span>
        </span>

        <span className={styles.address}>
          <span className={styles.label}>From</span>
          <Address value={formattedSenderAddress} className={styles.value} />
        </span>

        <ArrowSVG className={styles.arrowSvg} />

        <span className={styles.address}>
          <span className={styles.label}>To</span>
          <Address value={formattedReceiverAddress} className={styles.value} />
        </span>
      </div>

      {renderProgressBar?.()}

      <footer className={styles.footer}>
        {!isUndefined(estimatedFees) && <FeeAndTimeFooter isVaraNetwork={isVaraNetwork} feeValue={estimatedFees} />}
      </footer>
    </Modal>
  );
}

export { TransactionModal };
