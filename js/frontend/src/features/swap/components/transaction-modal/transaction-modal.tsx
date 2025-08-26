import { HexString } from '@gear-js/api';
import { useAccount, getVaraAddress } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { JSX } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { Address, FormattedBalance } from '@/components';
import { useTokens } from '@/context';
import { useEthAccount } from '@/hooks';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { FeeAndTimeFooter } from '../fee-and-time-footer';

import styles from './transaction-modal.module.scss';

type Props = {
  isVaraNetwork: boolean;
  amount: bigint;
  source: HexString;
  destination: HexString;
  receiver: string;
  estimatedFees: bigint;
  time: string;
  close: () => void;
  renderProgressBar: () => JSX.Element;
};

function TransactionModal({
  isVaraNetwork,
  amount,
  source,
  destination,
  receiver,
  estimatedFees,
  time,
  renderProgressBar,
  close,
}: Props) {
  const { getActiveToken } = useTokens();
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const SourceNetworkSVG = isVaraNetwork ? VaraSVG : EthSVG;
  const DestinationNetworkSVG = isVaraNetwork ? EthSVG : VaraSVG;

  const sourceToken = getActiveToken?.(source);
  const destinationToken = getActiveToken?.(destination);

  const sender = isVaraNetwork ? account!.decodedAddress : ethAccount.address!;
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
            {isVaraNetwork ? 'Vara' : 'Ethereum'}
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
            {isVaraNetwork ? 'Ethereum' : 'Vara'}
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

      {renderProgressBar()}

      <FeeAndTimeFooter isVaraNetwork={isVaraNetwork} feeValue={estimatedFees} time={time} />
    </Modal>
  );
}

export { TransactionModal };
