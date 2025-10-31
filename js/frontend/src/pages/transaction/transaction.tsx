import { HexString } from '@gear-js/api';
import { getVaraAddress } from '@gear-js/react-hooks';
import { PropsWithChildren } from 'react';
import { useParams } from 'react-router-dom';

import ArrowSVG from '@/assets/arrow.svg?react';
import { Container, Card, CopyButton, Address, FormattedBalance, TokenSVG, Skeleton } from '@/components';
import { useNetworkType, useTokens } from '@/context';
import { useOptimisticTxUpdate, useTransaction } from '@/features/history';
import { TransactionStatus } from '@/features/history/components/transaction-status';
import { NetworkEnum, StatusEnum } from '@/features/history/graphql/graphql';
import { PayVaraFeeButton, RelayTxButton, RelayTxNote } from '@/features/swap';
import { NETWORK } from '@/features/swap/consts';

import { Field } from './field';
import { SectionCard } from './section-card';
import { TransactionSkeleton } from './transaction-skeleton';
import styles from './transaction.module.scss';

type Params = {
  id: string;
};

const INDEXED_NETWORK_TO_NETWORK = {
  [NetworkEnum.Vara]: NETWORK.VARA,
  [NetworkEnum.Ethereum]: NETWORK.ETH,
} as const;

function useGetFullNetworkName() {
  const { NETWORK_PRESET } = useNetworkType();

  const indexedNetworkToNetworkName = {
    [NetworkEnum.Vara]: NETWORK_PRESET.NETWORK_NAME.VARA,
    [NetworkEnum.Ethereum]: NETWORK_PRESET.NETWORK_NAME.ETH,
  };

  return (network: NetworkEnum) => indexedNetworkToNetworkName[network];
}

const INDEXED_NETWORK_TO_NETWORK_NAME = {
  [NetworkEnum.Vara]: 'Vara',
  [NetworkEnum.Ethereum]: 'Ethereum',
} as const;

function useContractUrl(network: NetworkEnum) {
  const { NETWORK_PRESET } = useNetworkType();

  const networkToContractUrl = {
    [NetworkEnum.Vara]: (programId: string) =>
      `https://idea.gear-tech.io/programs/${programId}?node=${NETWORK_PRESET.NODE_ADDRESS}`,
    [NetworkEnum.Ethereum]: (programId: string) => `${NETWORK_PRESET.EXPLORER_URL.ETH}/address/${programId}`,
  };

  return networkToContractUrl[network];
}

function useAccountUrl(network: NetworkEnum) {
  const { NETWORK_PRESET } = useNetworkType();

  const networkToAccountUrl = {
    [NetworkEnum.Vara]: NETWORK_PRESET.EXPLORER_URL.VARA ? `${NETWORK_PRESET.EXPLORER_URL.VARA}/account` : undefined,
    [NetworkEnum.Ethereum]: `${NETWORK_PRESET.EXPLORER_URL.ETH}/address`,
  };

  return networkToAccountUrl[network];
}

function useTxUrl(network: NetworkEnum) {
  const { NETWORK_PRESET } = useNetworkType();

  const networkToTxUrl = {
    [NetworkEnum.Vara]: NETWORK_PRESET.EXPLORER_URL.VARA ? `${NETWORK_PRESET.EXPLORER_URL.VARA}/extrinsic` : undefined,
    [NetworkEnum.Ethereum]: `${NETWORK_PRESET.EXPLORER_URL.ETH}/tx`,
  };

  return networkToTxUrl[network];
}

function useBlockUrl(network: NetworkEnum) {
  const { NETWORK_PRESET } = useNetworkType();

  const networkToBlockUrl = {
    [NetworkEnum.Vara]: (blockNumber: string) =>
      `https://idea.gear-tech.io/explorer/${blockNumber}?node=${NETWORK_PRESET.ARCHIVE_NODE_ADDRESS}`,
    [NetworkEnum.Ethereum]: (blockNumber: string) => `${NETWORK_PRESET.EXPLORER_URL.ETH}/block/${blockNumber}`,
  };

  return networkToBlockUrl[network];
}

type ExplorerLinkProps = PropsWithChildren & {
  network: NetworkEnum;
  id: string;
  useUrl: typeof useAccountUrl | typeof useContractUrl | typeof useTxUrl | typeof useBlockUrl;
};

function ExplorerLink({ children, network, id, useUrl }: ExplorerLinkProps) {
  const urlOrGetUrl = useUrl(network);
  const url = typeof urlOrGetUrl === 'string' ? `${urlOrGetUrl}/${id}` : urlOrGetUrl?.(id);

  if (!url) return <span>{children}</span>;

  return (
    <a href={url} target="_blank" rel="noreferrer" className={styles.link}>
      {children}
    </a>
  );
}

function Transaction() {
  const { id } = useParams() as Params;

  const { getHistoryToken } = useTokens();
  const { data } = useTransaction(id);
  const optimisticTxUpdate = useOptimisticTxUpdate(id);
  const getFullNetworkName = useGetFullNetworkName();

  if (!data || !getHistoryToken) return <TransactionSkeleton />;

  const {
    status,
    source,
    destination,
    sourceNetwork,
    destNetwork,
    amount,
    sender,
    receiver,
    txHash,
    nonce,
    blockNumber,
    bridgingStartedAtBlock,
    bridgingStartedAtMessageId,
    timestamp,
    completedAt,
    completedAtBlock,
    completedAtTxHash,
  } = data;

  const sourceHex = source as HexString;
  const destinationHex = destination as HexString;

  const sourceToken = getHistoryToken(sourceHex, destinationHex);
  const destinationToken = getHistoryToken(destinationHex, sourceHex);

  const isVaraNetwork = sourceNetwork === NetworkEnum.Vara;
  const formattedSenderAddress = isVaraNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isVaraNetwork ? receiver : getVaraAddress(receiver);

  const isAwaitingPayment = status === StatusEnum.AwaitingPayment;

  return (
    <Container className={styles.container}>
      <header className={styles.header}>
        <div>
          <div className={styles.headingContainer}>
            <h1 className={styles.heading}>Transaction</h1>
            <TransactionStatus status={status} />
          </div>

          <p className={styles.subheading}>Cross-chain swap transaction information</p>
        </div>

        {isAwaitingPayment && (
          <div className={styles.sidebar}>
            <div className={styles.buttons}>
              {isVaraNetwork && (
                <PayVaraFeeButton nonce={nonce} onInBlock={() => optimisticTxUpdate(StatusEnum.Bridging)} />
              )}

              {isVaraNetwork ? (
                bridgingStartedAtBlock && (
                  <RelayTxButton.Vara
                    nonce={BigInt(nonce)}
                    blockNumber={bridgingStartedAtBlock}
                    onReceipt={optimisticTxUpdate}
                  />
                )
              ) : (
                <RelayTxButton.Eth
                  txHash={txHash as HexString}
                  blockNumber={BigInt(blockNumber)}
                  onInBlock={optimisticTxUpdate}
                />
              )}
            </div>

            {isVaraNetwork ? (
              bridgingStartedAtBlock && <RelayTxNote.Vara blockNumber={bridgingStartedAtBlock} />
            ) : (
              <RelayTxNote.Eth blockNumber={BigInt(blockNumber)} />
            )}
          </div>
        )}
      </header>

      <div className={styles.cards}>
        <SectionCard heading="Overview" gridContent={false}>
          <Card className={styles.transaction}>
            <div className={styles.token}>
              <TokenSVG symbol={sourceToken.symbol} network={INDEXED_NETWORK_TO_NETWORK[sourceNetwork]} />

              <div>
                <FormattedBalance
                  value={BigInt(amount)}
                  decimals={sourceToken.decimals}
                  symbol={sourceToken.displaySymbol}
                  truncated={false}
                  className={styles.amount}
                />

                <span className={styles.network}>{getFullNetworkName(sourceNetwork)}</span>
              </div>
            </div>

            <div className={styles.arrow}>
              <ArrowSVG />
            </div>

            <div className={styles.token}>
              <TokenSVG symbol={destinationToken.symbol} network={INDEXED_NETWORK_TO_NETWORK[destNetwork]} />

              <div>
                <FormattedBalance
                  value={BigInt(amount)}
                  decimals={destinationToken.decimals}
                  symbol={destinationToken.displaySymbol}
                  truncated={false}
                  className={styles.amount}
                />

                <span className={styles.network}>{getFullNetworkName(destNetwork)}</span>
              </div>
            </div>
          </Card>
        </SectionCard>

        <SectionCard heading="Addresses">
          <Field label="From Address">
            <ExplorerLink network={sourceNetwork} id={sender} useUrl={useAccountUrl}>
              <Address value={formattedSenderAddress} />
            </ExplorerLink>

            <CopyButton value={formattedSenderAddress} message="Sender address copied to clipboard" />
          </Field>

          <Field label="To Address">
            <ExplorerLink network={destNetwork} id={receiver} useUrl={useAccountUrl}>
              <Address value={formattedReceiverAddress} />
            </ExplorerLink>

            <CopyButton value={formattedReceiverAddress} message="Receiver address copied to clipboard" />
          </Field>

          <Field label={`${INDEXED_NETWORK_TO_NETWORK_NAME[sourceNetwork]} Contract Address`}>
            <ExplorerLink network={sourceNetwork} id={sourceHex} useUrl={useContractUrl}>
              <Address value={sourceHex} />
            </ExplorerLink>

            <CopyButton value={sourceHex} message="Source token address copied to clipboard" />
          </Field>

          <Field label={`${INDEXED_NETWORK_TO_NETWORK_NAME[destNetwork]} Contract Address`}>
            <ExplorerLink network={destNetwork} id={destinationHex} useUrl={useContractUrl}>
              <Address value={destinationHex} />
            </ExplorerLink>

            <CopyButton value={destinationHex} message="Destination token address copied to clipboard" />
          </Field>
        </SectionCard>

        <SectionCard heading="Identifiers">
          <Field label="Transaction Hash">
            <ExplorerLink network={sourceNetwork} id={txHash} useUrl={useTxUrl}>
              <Address value={txHash} />
            </ExplorerLink>

            <CopyButton value={txHash} message="Transaction hash copied to clipboard" />
          </Field>

          <Field label="Transaction Nonce">
            <Address value={nonce} />
            <CopyButton value={nonce} message="Transaction nonce copied to clipboard" />
          </Field>

          <Field label="Block Number">
            <ExplorerLink network={sourceNetwork} id={blockNumber.toString()} useUrl={useBlockUrl}>
              #{blockNumber}
            </ExplorerLink>
          </Field>

          <Field label="Vara Block Number">
            {bridgingStartedAtBlock ? (
              <ExplorerLink network={NetworkEnum.Vara} id={bridgingStartedAtBlock.toString()} useUrl={useBlockUrl}>
                #{bridgingStartedAtBlock}
              </ExplorerLink>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>

          <Field label="Vara Message ID">
            {bridgingStartedAtMessageId ? (
              <>
                <Address value={bridgingStartedAtMessageId} />
                <CopyButton value={bridgingStartedAtMessageId} message="Message ID copied to clipboard" />
              </>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>
        </SectionCard>

        <SectionCard heading="Timings">
          <Field label="Initiated At">
            <span>{new Date(timestamp).toLocaleString()}</span>
          </Field>

          <Field label="Completed At">
            {completedAt ? <span>{new Date(completedAt).toLocaleString()}</span> : <Skeleton width="5rem" disabled />}
          </Field>

          <Field label="Completed At Block">
            {completedAtBlock ? (
              <ExplorerLink network={destNetwork} id={completedAtBlock} useUrl={useBlockUrl}>
                #{completedAtBlock.toLocaleString()}
              </ExplorerLink>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>

          <Field label="Completed At Transaction Hash">
            {completedAtTxHash ? (
              <>
                <ExplorerLink network={destNetwork} id={completedAtTxHash} useUrl={useTxUrl}>
                  <Address value={completedAtTxHash} />
                </ExplorerLink>

                <CopyButton value={completedAtTxHash} message="Completion transaction hash copied to clipboard" />
              </>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>
        </SectionCard>
      </div>
    </Container>
  );
}

export { Transaction };
