import { HexString } from '@gear-js/api';
import { getVaraAddress, useAccount, useAlert, useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';
import { Button, Modal } from '@gear-js/vara-ui';
import { useQueryClient } from '@tanstack/react-query';
import { JSX } from 'react';

import { Address, CopyButton, FeeAndTimeFooter, FormattedBalance, LinkButton } from '@/components';
import { useTokens } from '@/context';
import { getAddressToTokenKey } from '@/context/tokens';
import { BridgingPaymentProgram, CONTRACT_ADDRESS } from '@/features/swap/consts';
import { useVaraFee } from '@/features/swap/hooks';
import { cx, getErrorMessage, isUndefined, getTruncatedText } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { EXPLORER_URL, NETWORK_SVG } from '../../consts';
import { Network, Status, Transfer } from '../../types';
import { TransactionDate } from '../transaction-date';
import { TransactionStatus } from '../transaction-status';

import styles from './transaction-modal.module.scss';

type Props = Pick<
  Transfer,
  'amount' | 'destination' | 'source' | 'sourceNetwork' | 'destNetwork' | 'sender' | 'receiver'
> & {
  txHash?: Transfer['txHash'];
  timestamp?: Transfer['timestamp'];
  status?: Transfer['status'];
  nonce?: Transfer['nonce'];
  blockNumber?: Transfer['blockNumber'];
  estimatedFees?: bigint;
  close: () => void;
  renderProgressBar?: () => JSX.Element;
};

// TODO: reuse hook from @features/swap
function usePayFee() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: CONTRACT_ADDRESS.BRIDGING_PAYMENT,
  });

  return useSendProgramTransaction({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'payFees',
  });
}

function TransactionModal({
  status,
  txHash,
  sourceNetwork,
  destNetwork,
  timestamp,
  amount,
  source,
  destination,
  sender,
  receiver,
  nonce,
  blockNumber,
  estimatedFees,
  renderProgressBar,
  close,
}: Props) {
  const { addressToToken } = useTokens();
  const isVaraNetwork = sourceNetwork === Network.Vara;

  const { account } = useAccount();
  const payFee = usePayFee();
  const alert = useAlert();
  const queryClient = useQueryClient();

  const { bridgingFee: varaBridgingFee } = useVaraFee();
  const rawNonce = isVaraNetwork && nonce ? `0x${nonce.padStart(64, '0')}` : nonce;
  const isPayFeeButtonVisible = nonce && account?.decodedAddress === sender && status === Status.AwaitingPayment;

  const explorerUrl = `${EXPLORER_URL[sourceNetwork]}/${isVaraNetwork ? blockNumber : `tx/${txHash}`}`;
  const SourceNetworkSVG = NETWORK_SVG[sourceNetwork];
  const DestinationNetworkSVG = NETWORK_SVG[destNetwork];

  const sourceToken = addressToToken?.[getAddressToTokenKey(source as HexString, destination as HexString)];
  const destinationToken = addressToToken?.[getAddressToTokenKey(destination as HexString, source as HexString)];

  const formattedSenderAddress = isVaraNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isVaraNetwork ? receiver : getVaraAddress(receiver);

  const handlePayFeeButtonClick = () => {
    if (!rawNonce) throw new Error('Nonce is not found');
    if (isUndefined(varaBridgingFee.value)) throw new Error('Fee is not found');

    payFee
      .sendTransactionAsync({ args: [rawNonce], value: varaBridgingFee.value })
      .then(() => {
        close();
        alert.success('Fee paid successfully');

        return queryClient.invalidateQueries({ queryKey: ['transactions'] });
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));
  };

  return (
    <Modal
      heading="Transaction Details"
      headerAddon={status && <TransactionStatus status={status} />}
      close={close}
      maxWidth="large">
      {(txHash || rawNonce || timestamp) && (
        <header className={styles.header}>
          <div className={styles.txHashAndNonce}>
            {txHash && (
              <p className={styles.txHash}>
                {!isVaraNetwork ? (
                  <a href={explorerUrl} target="_blank" rel="noreferrer">
                    {getTruncatedText(txHash)}
                  </a>
                ) : (
                  getTruncatedText(txHash)
                )}

                <CopyButton value={txHash} message="Transaction hash copied" />
              </p>
            )}

            {rawNonce && (
              <p className={styles.nonce}>
                Nonce:
                <Address value={rawNonce} prefixLength={4} />
                <CopyButton value={rawNonce} />
              </p>
            )}
          </div>

          {timestamp && <TransactionDate timestamp={timestamp} className={styles.date} />}
        </header>
      )}

      <div className={cx(styles.pairs, renderProgressBar && styles.loading)}>
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

        {((isVaraNetwork ? Boolean(blockNumber) : txHash) || isPayFeeButtonVisible) && (
          <div className={styles.buttons}>
            {(isVaraNetwork ? Boolean(blockNumber) : txHash) && (
              <LinkButton
                type="external"
                to={explorerUrl}
                text={isVaraNetwork ? 'View Block in Explorer' : 'View in Explorer'}
                color="contrast"
                size="small"
                block
              />
            )}

            {isPayFeeButtonVisible && (
              <Button
                text="Pay Fee"
                size="small"
                isLoading={payFee.isPending}
                onClick={handlePayFeeButtonClick}
                block
              />
            )}
          </div>
        )}
      </footer>
    </Modal>
  );
}

export { TransactionModal };
