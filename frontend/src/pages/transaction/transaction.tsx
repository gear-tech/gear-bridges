import { HexString } from '@gear-js/api';
import { PropsWithChildren } from 'react';
import { useParams } from 'react-router-dom';

import { Container, Card, CopyButton, Address, FormattedBalance, TokenSVG, Skeleton } from '@/components';
import { useTokens } from '@/context';
import { useTransaction } from '@/features/history';
import ArrowSVG from '@/features/history/assets/arrow.svg?react';
import { TransactionStatus } from '@/features/history/components/transaction-status';
import { NetworkEnum } from '@/features/history/graphql/graphql';
import { NETWORK } from '@/features/swap/consts';

import styles from './transaction.module.scss';

type FieldProps = PropsWithChildren & {
  label: string;
};

function Field({ label, children }: FieldProps) {
  return (
    <div className={styles.field}>
      <span className={styles.label}>{label}:</span>
      <Card className={styles.value}>{children}</Card>
    </div>
  );
}

type SectionCardProps = PropsWithChildren & {
  heading: string;
  gridContent?: boolean;
};

function SectionCard({ heading, children, gridContent = true }: SectionCardProps) {
  return (
    <Card className={styles.section}>
      <h2 className={styles.heading}>{heading}</h2>
      <div className={gridContent ? styles.content : undefined}>{children}</div>
    </Card>
  );
}

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

function Transaction() {
  const { id } = useParams() as Params;
  const { addressToToken } = useTokens();
  const { data } = useTransaction(id);

  if (!data || !addressToToken) return;

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

  const sourceToken = addressToToken[sourceHex];
  const destinationToken = addressToToken[destinationHex];

  return (
    <Container className={styles.container}>
      <header className={styles.header}>
        <div>
          <h1 className={styles.heading}>Transaction</h1>
          <p className={styles.subheading}>Cross-chain swap transaction information</p>
        </div>

        <TransactionStatus status={status} />
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
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={sender} />
            </a>

            <CopyButton value={sender} message="Sender address copied to clipboard" />
          </Field>

          <Field label="To Address">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={receiver} />
            </a>

            <CopyButton value={receiver} message="Receiver address copied to clipboard" />
          </Field>

          <Field label={`${INDEXED_NETWORK_TO_NETWORK_NAME[sourceNetwork]} Contract Address`}>
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={sourceHex} />
            </a>

            <CopyButton value={sourceHex} message="Source token address copied to clipboard" />
          </Field>

          <Field label={`${INDEXED_NETWORK_TO_NETWORK_NAME[destNetwork]} Contract Address`}>
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={destinationHex} />
            </a>

            <CopyButton value={destinationHex} message="Destination token address copied to clipboard" />
          </Field>
        </SectionCard>

        <SectionCard heading="Identifiers">
          <Field label="Transaction Hash">
            <a href="/" className={styles.link} target="_blank" rel="noreferrer">
              <Address value={txHash} />
            </a>

            <CopyButton value={txHash} message="Transaction hash copied to clipboard" />
          </Field>

          <Field label="Transaction Nonce">
            <Address value={nonce} />
            <CopyButton value={nonce} message="Transaction nonce copied to clipboard" />
          </Field>

          <Field label="Block Number">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              #{blockNumber.toLocaleString()}
            </a>
          </Field>

          <Field label="Vara Block Number">
            {bridgingStartedAtBlock ? (
              <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
                #{bridgingStartedAtBlock.toLocaleString()}
              </a>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>

          <Field label="Vara Message ID">
            {bridgingStartedAtMessageId ? (
              <>
                <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
                  <Address value={bridgingStartedAtMessageId} />
                </a>

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
              <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
                #{completedAtBlock.toLocaleString()}
              </a>
            ) : (
              <Skeleton width="5rem" disabled />
            )}
          </Field>

          <Field label="Completed At Transaction Hash">
            {completedAtTxHash ? (
              <>
                <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
                  <Address value={completedAtTxHash} />
                </a>
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
