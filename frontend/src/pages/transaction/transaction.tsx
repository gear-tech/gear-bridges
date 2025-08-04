import { PropsWithChildren } from 'react';
import { useParams } from 'react-router-dom';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { Container, Card, CopyButton, Address, FormattedBalance, TokenSVG, LinkButton } from '@/components';
import { useTransaction } from '@/features/history';
import ArrowSVG from '@/features/history/assets/arrow.svg?react';
import { TransactionStatus } from '@/features/history/components/transaction-status';

import styles from './transaction.module.scss';

// Mock data for placeholder - will be replaced with real data later
const mockTransactionData = {
  // Transaction Identifiers
  nonce: '0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef',
  txHash: '0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890',
  blockNumber: 12345678,

  // Vara-specific
  varaBlockNumber: 8765432,
  messageId: '0x9876543210fedcba9876543210fedcba9876543210fedcba9876543210fedcba',

  // Timing Information
  timestamp: new Date(),
  completedAt: {
    blockNumber: 12345680,
    timestamp: new Date(Date.now() + 300000), // 5 minutes later
    txHash: '0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321',
  },

  // Token Information
  sourceToken: {
    address: '0x1234567890123456789012345678901234567890',
    symbol: 'USDC',
    decimals: 6,
  },
  destinationToken: {
    address: '0x0987654321098765432109876543210987654321',
    symbol: 'vUSDC',
    decimals: 6,
  },

  // Transaction Participants
  sender: '0xabcdef1234567890abcdef1234567890abcdef12',
  receiver: '5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY',
  amount: '1000000000', // 1000 USDC with 6 decimals

  // Transaction Status
  status: 'Completed' as const, // "Awaiting Payment" | "Bridging" | "Completed"

  // Networks
  sourceNetwork: 'Ethereum' as const,
  destNetwork: 'Vara' as const,
};

type Params = {
  id: string;
};

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

function Transaction() {
  const { id } = useParams() as Params;
  const { data } = useTransaction(id);

  if (!data) return;

  return (
    <Container className={styles.container}>
      <header className={styles.header}>
        <div>
          <h1 className={styles.heading}>Transaction</h1>
          <p className={styles.subheading}>Cross-chain swap transaction information</p>
        </div>

        <TransactionStatus status={data.status} />
      </header>

      <div className={styles.cards}>
        <SectionCard heading="Overview" gridContent={false}>
          <Card className={styles.transaction}>
            <div className={styles.token}>
              <TokenSVG symbol={mockTransactionData.sourceToken.symbol} network="eth" sizes={[48, 28]} />

              <div>
                <FormattedBalance
                  value={BigInt(mockTransactionData.amount)}
                  decimals={mockTransactionData.sourceToken.decimals}
                  symbol={mockTransactionData.sourceToken.symbol}
                  className={styles.amount}
                />

                <span className={styles.network}>{mockTransactionData.sourceNetwork}</span>
              </div>
            </div>

            <div className={styles.arrow}>
              <ArrowSVG />
            </div>

            <div className={styles.token}>
              <TokenSVG symbol={mockTransactionData.destinationToken.symbol} network="vara" sizes={[48, 28]} />

              <div>
                <FormattedBalance
                  value={BigInt(mockTransactionData.amount)}
                  decimals={mockTransactionData.sourceToken.decimals}
                  symbol={mockTransactionData.sourceToken.symbol}
                  className={styles.amount}
                />

                <span className={styles.network}>{mockTransactionData.destNetwork}</span>
              </div>
            </div>
          </Card>
        </SectionCard>

        <SectionCard heading="Addresses">
          <Field label="From">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={mockTransactionData.sender} />
            </a>

            <CopyButton value={mockTransactionData.sender} message="Sender address copied to clipboard" />
          </Field>

          <Field label="Contract Address">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={mockTransactionData.sourceToken.address} />
            </a>

            <CopyButton
              value={mockTransactionData.sourceToken.address}
              message="Source token address copied to clipboard"
            />
          </Field>

          <Field label="To">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={mockTransactionData.receiver} />
            </a>

            <CopyButton value={mockTransactionData.receiver} message="Receiver address copied to clipboard" />
          </Field>

          <Field label="Contract Address">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={mockTransactionData.destinationToken.address} />
            </a>

            <CopyButton
              value={mockTransactionData.destinationToken.address}
              message="Destination token address copied to clipboard"
            />
          </Field>
        </SectionCard>

        <SectionCard heading="Identifiers">
          <Field label="Transaction Hash">
            <a href="/" className={styles.link} target="_blank" rel="noreferrer">
              <Address value={mockTransactionData.txHash} />
            </a>

            <CopyButton value={mockTransactionData.txHash} message="Transaction hash copied to clipboard" />
          </Field>

          <Field label="Transaction Nonce">
            <Address value={mockTransactionData.nonce} />
            <CopyButton value={mockTransactionData.nonce} message="Transaction nonce copied to clipboard" />
          </Field>

          <Field label="Block Number">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              #{mockTransactionData.blockNumber.toLocaleString()}
            </a>
          </Field>

          <Field label="Vara Block Number">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              #{mockTransactionData.varaBlockNumber.toLocaleString()}
            </a>
          </Field>

          <Field label="Vara Message ID">
            <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
              <Address value={mockTransactionData.messageId} />
            </a>

            <CopyButton value={mockTransactionData.messageId} message="Message ID copied to clipboard" />
          </Field>
        </SectionCard>

        <SectionCard heading="Timings">
          <Field label="Initiated At">
            <span>{new Date(data.timestamp).toLocaleString()}</span>
          </Field>

          {data.completedAt && (
            <>
              <Field label="Completed At">
                <span>{new Date(data.completedAt).toLocaleString()}</span>
              </Field>

              <Field label="Completed At Block">
                <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
                  #{mockTransactionData.completedAt.blockNumber.toLocaleString()}
                </a>
              </Field>

              <Field label="Completed At Transaction Hash">
                <a href="/" target="_blank" rel="noreferrer" className={styles.link}>
                  <Address value={mockTransactionData.completedAt.txHash} />
                </a>

                <CopyButton
                  value={mockTransactionData.completedAt.txHash}
                  message="Completion transaction hash copied to clipboard"
                />
              </Field>
            </>
          )}
        </SectionCard>
      </div>
    </Container>
  );
}

export { Transaction };
