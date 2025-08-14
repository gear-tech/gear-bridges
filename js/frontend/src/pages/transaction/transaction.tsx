import { HexString } from '@gear-js/api';
import { getVaraAddress, useAccount } from '@gear-js/react-hooks';
import { PropsWithChildren } from 'react';
import { useParams } from 'react-router-dom';

import { Container, Card, CopyButton, Address, FormattedBalance, TokenSVG, Skeleton } from '@/components';
import { useTokens } from '@/context';
import { getAddressToTokenKey } from '@/context/tokens';
import { useTransaction } from '@/features/history';
import ArrowSVG from '@/features/history/assets/arrow.svg?react';
import { TransactionStatus } from '@/features/history/components/transaction-status';
import { NetworkEnum, StatusEnum } from '@/features/history/graphql/graphql';
import { PayVaraFeeButton } from '@/features/swap';
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

const INDEXED_NETWORK_TO_FULL_NETWORK_NAME = {
  [NetworkEnum.Vara]: 'Vara Testnet',
  [NetworkEnum.Ethereum]: 'Ethereum Hoodi',
} as const;

const INDEXED_NETWORK_TO_NETWORK_NAME = {
  [NetworkEnum.Vara]: 'Vara',
  [NetworkEnum.Ethereum]: 'Ethereum',
} as const;

const CONTRACT_URL = {
  [NetworkEnum.Vara]: (programId: string) =>
    `https://idea.gear-tech.io/programs/${programId}?node=wss://testnet.vara.network`,
  [NetworkEnum.Ethereum]: (programId: string) => `https://hoodi.etherscan.io/address/${programId}`,
} as const;

const ACCOUNT_URL = {
  [NetworkEnum.Vara]: () => undefined,
  [NetworkEnum.Ethereum]: (address: string) => `https://hoodi.etherscan.io/address/${address}`,
} as const;

const TX_URL = {
  [NetworkEnum.Vara]: () => undefined,
  [NetworkEnum.Ethereum]: `https://hoodi.etherscan.io/tx`,
} as const;

const BLOCK_URL = {
  [NetworkEnum.Vara]: (blockNumber: string) =>
    `https://idea.gear-tech.io/explorer/${blockNumber}?node=wss://testnet-archive.vara.network`,
  [NetworkEnum.Ethereum]: (blockNumber: string) => `https://hoodi.etherscan.io/block/${blockNumber}`,
} as const;

type ExplorerLinkProps = PropsWithChildren & {
  network: NetworkEnum;
  id: string;
  urls: typeof TX_URL | typeof BLOCK_URL | typeof ACCOUNT_URL;
};

function ExplorerLink({ children, network, id, urls }: ExplorerLinkProps) {
  const urlOrGetUrl = urls[network];
  const url = typeof urlOrGetUrl === 'string' ? `${urlOrGetUrl}/${id}` : urlOrGetUrl(id);

  if (!url) return <span>{children}</span>;

  return (
    <a href={url} target="_blank" rel="noreferrer" className={styles.link}>
      {children}
    </a>
  );
}

function Transaction() {
  const { account } = useAccount();
  const { id } = useParams() as Params;

  const { addressToToken } = useTokens();
  const { data } = useTransaction(id);

  if (!data || !addressToToken) return <TransactionSkeleton />;

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

  const sourceToken = addressToToken[getAddressToTokenKey(sourceHex, destinationHex)];
  const destinationToken = addressToToken[getAddressToTokenKey(destinationHex, sourceHex)];

  const isVaraNetwork = sourceNetwork === NetworkEnum.Vara;
  const formattedSenderAddress = isVaraNetwork ? getVaraAddress(sender) : sender;
  const formattedReceiverAddress = isVaraNetwork ? receiver : getVaraAddress(receiver);

  const isPayFeeButtonVisible = account?.decodedAddress === sender && status === StatusEnum.AwaitingPayment;
  const rawNonce = isVaraNetwork ? `0x${nonce.padStart(64, '0')}` : nonce;

  return (
    <Container className={styles.container}>
      <header className={styles.header}>
        <div>
          <h1 className={styles.heading}>Transaction</h1>
          <p className={styles.subheading}>Cross-chain swap transaction information</p>
        </div>

        <div className={styles.sidebar}>
          <TransactionStatus status={status} />

          {isPayFeeButtonVisible && <PayVaraFeeButton transactionId={id} nonce={rawNonce} />}
        </div>
      </header>

      <div className={styles.cards}>
        <SectionCard heading="Overview" gridContent={false}>
          <Card className={styles.transaction}>
            <div className={styles.token}>
              <TokenSVG
                symbol={sourceToken.symbol}
                network={INDEXED_NETWORK_TO_NETWORK[sourceNetwork]}
                sizes={[48, 28]}
              />

              <div>
                <FormattedBalance
                  value={BigInt(amount)}
                  decimals={sourceToken.decimals}
                  symbol={sourceToken.displaySymbol}
                  className={styles.amount}
                />

                <span className={styles.network}>{INDEXED_NETWORK_TO_FULL_NETWORK_NAME[sourceNetwork]}</span>
              </div>
            </div>

            <div className={styles.arrow}>
              <ArrowSVG />
            </div>

            <div className={styles.token}>
              <TokenSVG
                symbol={destinationToken.symbol}
                network={INDEXED_NETWORK_TO_NETWORK[destNetwork]}
                sizes={[48, 28]}
              />

              <div>
                <FormattedBalance
                  value={BigInt(amount)}
                  decimals={destinationToken.decimals}
                  symbol={destinationToken.displaySymbol}
                  className={styles.amount}
                />

                <span className={styles.network}>{INDEXED_NETWORK_TO_FULL_NETWORK_NAME[destNetwork]}</span>
              </div>
            </div>
          </Card>
        </SectionCard>

        <SectionCard heading="Addresses">
          <Field label="From Address">
            <ExplorerLink network={sourceNetwork} id={sender} urls={ACCOUNT_URL}>
              <Address value={formattedSenderAddress} />
            </ExplorerLink>

            <CopyButton value={formattedSenderAddress} message="Sender address copied to clipboard" />
          </Field>

          <Field label="To Address">
            <ExplorerLink network={destNetwork} id={receiver} urls={ACCOUNT_URL}>
              <Address value={formattedReceiverAddress} />
            </ExplorerLink>

            <CopyButton value={formattedReceiverAddress} message="Receiver address copied to clipboard" />
          </Field>

          <Field label={`${INDEXED_NETWORK_TO_NETWORK_NAME[sourceNetwork]} Contract Address`}>
            <ExplorerLink network={sourceNetwork} id={sourceHex} urls={CONTRACT_URL}>
              <Address value={sourceHex} />
            </ExplorerLink>

            <CopyButton value={sourceHex} message="Source token address copied to clipboard" />
          </Field>

          <Field label={`${INDEXED_NETWORK_TO_NETWORK_NAME[destNetwork]} Contract Address`}>
            <ExplorerLink network={destNetwork} id={destinationHex} urls={CONTRACT_URL}>
              <Address value={destinationHex} />
            </ExplorerLink>

            <CopyButton value={destinationHex} message="Destination token address copied to clipboard" />
          </Field>
        </SectionCard>

        <SectionCard heading="Identifiers">
          <Field label="Transaction Hash">
            <ExplorerLink network={sourceNetwork} id={txHash} urls={TX_URL}>
              <Address value={txHash} />
            </ExplorerLink>

            <CopyButton value={txHash} message="Transaction hash copied to clipboard" />
          </Field>

          <Field label="Transaction Nonce">
            <Address value={rawNonce} />
            <CopyButton value={rawNonce} message="Transaction nonce copied to clipboard" />
          </Field>

          <Field label="Block Number">
            <ExplorerLink network={sourceNetwork} id={blockNumber.toString()} urls={BLOCK_URL}>
              #{blockNumber}
            </ExplorerLink>
          </Field>

          <Field label="Vara Block Number">
            {bridgingStartedAtBlock ? (
              <ExplorerLink network={NetworkEnum.Vara} id={bridgingStartedAtBlock.toString()} urls={BLOCK_URL}>
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
              <ExplorerLink network={destNetwork} id={completedAtBlock} urls={BLOCK_URL}>
                #{completedAtBlock.toLocaleString()}
              </ExplorerLink>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>

          <Field label="Completed At Transaction Hash">
            {completedAtTxHash ? (
              <>
                <ExplorerLink network={destNetwork} id={completedAtTxHash} urls={TX_URL}>
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
