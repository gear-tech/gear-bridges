import { useParams } from 'react-router-dom';

import ClockSVG from '@/assets/clock.svg?react';
import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { Container, Card, CopyButton, Address, FormattedBalance, TokenSVG, LinkButton } from '@/components';
import { useTransaction } from '@/features/history';

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

function Transaction() {
  const { id } = useParams() as Params;
  const { data } = useTransaction(id);
  console.log('data: ', data);

  const formatDate = (date: Date) => {
    return {
      readable: date.toLocaleDateString('en-US', {
        year: 'numeric',
        month: 'long',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      }),
      utc: date.toISOString(),
    };
  };

  const getStatusIcon = (status: string) => {
    switch (status) {
      case 'Completed':
        return <div className={styles.statusCompleted}>✓</div>;
      case 'Bridging':
        return <ClockSVG className={styles.statusIcon} />;
      case 'Awaiting Payment':
        return <ClockSVG className={styles.statusIcon} />;
      default:
        return <ClockSVG className={styles.statusIcon} />;
    }
  };

  const getStatusClass = (status: string) => {
    switch (status) {
      case 'Completed':
        return styles.completed;
      case 'Bridging':
        return styles.bridging;
      case 'Awaiting Payment':
        return styles.awaitingPayment;
      default:
        return styles.bridging;
    }
  };

  const getNetworkIcon = (network: string) => {
    return network === 'Ethereum' ? <EthSVG /> : <VaraSVG />;
  };

  const getExplorerUrl = (type: 'tx' | 'block' | 'address', value: string, network: string) => {
    const baseUrl = network === 'Ethereum' ? 'https://holesky.etherscan.io' : 'https://idea.gear-tech.io';

    if (network === 'Ethereum') {
      switch (type) {
        case 'tx':
          return `${baseUrl}/tx/${value}`;
        case 'block':
          return `${baseUrl}/block/${value}`;
        case 'address':
          return `${baseUrl}/address/${value}`;
        default:
          return baseUrl;
      }
    } else {
      switch (type) {
        case 'tx':
          return `${baseUrl}/message/${value}`;
        case 'block':
          return `${baseUrl}/block/${value}`;
        case 'address':
          return `${baseUrl}/account/${value}`;
        default:
          return baseUrl;
      }
    }
  };

  const initTimestamp = formatDate(mockTransactionData.timestamp);
  const completedTimestamp = formatDate(mockTransactionData.completedAt.timestamp);

  return (
    <Container>
      <div className={styles.container}>
        <header className={styles.header}>
          <h1 className={styles.title}>Transaction Details</h1>
          <div className={`${styles.status} ${getStatusClass(mockTransactionData.status)}`}>
            {getStatusIcon(mockTransactionData.status)}
            <span>{mockTransactionData.status}</span>
          </div>
        </header>

        <div className={styles.content}>
          {/* Transaction Identifiers */}
          <Card className={styles.section}>
            <h2 className={styles.sectionTitle}>Transaction Identifiers</h2>
            <div className={styles.fields}>
              <div className={styles.field}>
                <span className={styles.label}>Transaction Hash</span>
                <div className={styles.fieldContent}>
                  <LinkButton
                    type="external"
                    to={getExplorerUrl('tx', mockTransactionData.txHash, mockTransactionData.sourceNetwork)}
                    className={styles.link}>
                    <Address value={mockTransactionData.txHash} />
                  </LinkButton>
                  <CopyButton value={mockTransactionData.txHash} message="Transaction hash copied to clipboard" />
                </div>
              </div>

              <div className={styles.field}>
                <span className={styles.label}>Transaction Nonce</span>
                <div className={styles.fieldContent}>
                  <Address value={mockTransactionData.nonce} />
                  <CopyButton value={mockTransactionData.nonce} message="Transaction nonce copied to clipboard" />
                </div>
              </div>

              <div className={styles.field}>
                <span className={styles.label}>Block Number</span>
                <div className={styles.fieldContent}>
                  <LinkButton
                    type="external"
                    to={getExplorerUrl(
                      'block',
                      mockTransactionData.blockNumber.toString(),
                      mockTransactionData.sourceNetwork,
                    )}
                    className={styles.link}>
                    #{mockTransactionData.blockNumber.toLocaleString()}
                  </LinkButton>
                </div>
              </div>
            </div>
          </Card>

          {/* Timing Information */}
          <Card className={styles.section}>
            <h2 className={styles.sectionTitle}>Timing Information</h2>
            <div className={styles.fields}>
              <div className={styles.field}>
                <span className={styles.label}>Initiated At</span>
                <div className={styles.fieldContent}>
                  <div className={styles.dateTime}>
                    <span className={styles.readableDate}>{initTimestamp.readable}</span>
                    <span className={styles.utcDate}>UTC: {initTimestamp.utc}</span>
                  </div>
                </div>
              </div>

              {mockTransactionData.status === 'Completed' && (
                <div className={styles.field}>
                  <span className={styles.label}>Completed At</span>
                  <div className={styles.fieldContent}>
                    <div className={styles.completedInfo}>
                      <div className={styles.dateTime}>
                        <span className={styles.readableDate}>{completedTimestamp.readable}</span>
                        <span className={styles.utcDate}>UTC: {completedTimestamp.utc}</span>
                      </div>
                      <div className={styles.completedDetails}>
                        <LinkButton
                          type="external"
                          to={getExplorerUrl(
                            'block',
                            mockTransactionData.completedAt.blockNumber.toString(),
                            mockTransactionData.destNetwork,
                          )}
                          className={styles.link}>
                          Block #{mockTransactionData.completedAt.blockNumber.toLocaleString()}
                        </LinkButton>
                        <LinkButton
                          type="external"
                          to={getExplorerUrl(
                            'tx',
                            mockTransactionData.completedAt.txHash,
                            mockTransactionData.destNetwork,
                          )}
                          className={styles.link}>
                          <Address value={mockTransactionData.completedAt.txHash} />
                        </LinkButton>
                        <CopyButton
                          value={mockTransactionData.completedAt.txHash}
                          message="Completion transaction hash copied to clipboard"
                        />
                      </div>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </Card>

          {/* Token Information */}
          <Card className={styles.section}>
            <h2 className={styles.sectionTitle}>Token Information</h2>
            <div className={styles.tokenPair}>
              <div className={styles.tokenInfo}>
                <div className={styles.tokenHeader}>
                  <TokenSVG symbol={mockTransactionData.sourceToken.symbol} network="eth" sizes={[32, 20]} />
                  <div>
                    <h3 className={styles.tokenSymbol}>{mockTransactionData.sourceToken.symbol}</h3>
                    <span className={styles.networkBadge}>
                      {getNetworkIcon(mockTransactionData.sourceNetwork)}
                      {mockTransactionData.sourceNetwork}
                    </span>
                  </div>
                </div>
                <div className={styles.tokenAddress}>
                  <span className={styles.label}>Contract Address</span>
                  <div className={styles.fieldContent}>
                    <LinkButton
                      type="external"
                      to={getExplorerUrl(
                        'address',
                        mockTransactionData.sourceToken.address,
                        mockTransactionData.sourceNetwork,
                      )}
                      className={styles.link}>
                      <Address value={mockTransactionData.sourceToken.address} />
                    </LinkButton>
                    <CopyButton
                      value={mockTransactionData.sourceToken.address}
                      message="Source token address copied to clipboard"
                    />
                  </div>
                </div>
              </div>

              <div className={styles.arrowContainer}>
                <div className={styles.arrow}>→</div>
              </div>

              <div className={styles.tokenInfo}>
                <div className={styles.tokenHeader}>
                  <TokenSVG symbol={mockTransactionData.destinationToken.symbol} network="vara" sizes={[32, 20]} />
                  <div>
                    <h3 className={styles.tokenSymbol}>{mockTransactionData.destinationToken.symbol}</h3>
                    <span className={styles.networkBadge}>
                      {getNetworkIcon(mockTransactionData.destNetwork)}
                      {mockTransactionData.destNetwork}
                    </span>
                  </div>
                </div>
                <div className={styles.tokenAddress}>
                  <span className={styles.label}>Contract Address</span>
                  <div className={styles.fieldContent}>
                    <LinkButton
                      type="external"
                      to={getExplorerUrl(
                        'address',
                        mockTransactionData.destinationToken.address,
                        mockTransactionData.destNetwork,
                      )}
                      className={styles.link}>
                      <Address value={mockTransactionData.destinationToken.address} />
                    </LinkButton>
                    <CopyButton
                      value={mockTransactionData.destinationToken.address}
                      message="Destination token address copied to clipboard"
                    />
                  </div>
                </div>
              </div>
            </div>
          </Card>

          {/* Transaction Participants */}
          <Card className={styles.section}>
            <h2 className={styles.sectionTitle}>Transaction Participants</h2>
            <div className={styles.fields}>
              <div className={styles.field}>
                <span className={styles.label}>Sender</span>
                <div className={styles.fieldContent}>
                  <div className={styles.participantInfo}>
                    <span className={styles.networkBadge}>
                      {getNetworkIcon(mockTransactionData.sourceNetwork)}
                      {mockTransactionData.sourceNetwork}
                    </span>
                    <LinkButton
                      type="external"
                      to={getExplorerUrl('address', mockTransactionData.sender, mockTransactionData.sourceNetwork)}
                      className={styles.link}>
                      <Address value={mockTransactionData.sender} />
                    </LinkButton>
                    <CopyButton value={mockTransactionData.sender} message="Sender address copied to clipboard" />
                  </div>
                </div>
              </div>

              <div className={styles.field}>
                <span className={styles.label}>Receiver</span>
                <div className={styles.fieldContent}>
                  <div className={styles.participantInfo}>
                    <span className={styles.networkBadge}>
                      {getNetworkIcon(mockTransactionData.destNetwork)}
                      {mockTransactionData.destNetwork}
                    </span>
                    <LinkButton
                      type="external"
                      to={getExplorerUrl('address', mockTransactionData.receiver, mockTransactionData.destNetwork)}
                      className={styles.link}>
                      <Address value={mockTransactionData.receiver} />
                    </LinkButton>
                    <CopyButton value={mockTransactionData.receiver} message="Receiver address copied to clipboard" />
                  </div>
                </div>
              </div>

              <div className={styles.field}>
                <span className={styles.label}>Amount</span>
                <div className={styles.fieldContent}>
                  <div className={styles.amount}>
                    <FormattedBalance
                      value={BigInt(mockTransactionData.amount)}
                      decimals={mockTransactionData.sourceToken.decimals}
                      symbol={mockTransactionData.sourceToken.symbol}
                      className={styles.amountValue}
                    />
                  </div>
                </div>
              </div>
            </div>
          </Card>

          {/* Vara-Specific Information - Show for demo purposes */}
          <Card className={styles.section}>
            <h2 className={styles.sectionTitle}>Vara Network Information</h2>
            <div className={styles.fields}>
              <div className={styles.field}>
                <span className={styles.label}>Vara Block Number</span>
                <div className={styles.fieldContent}>
                  <LinkButton
                    type="external"
                    to={getExplorerUrl('block', mockTransactionData.varaBlockNumber.toString(), 'Vara')}
                    className={styles.link}>
                    #{mockTransactionData.varaBlockNumber.toLocaleString()}
                  </LinkButton>
                </div>
              </div>

              <div className={styles.field}>
                <span className={styles.label}>Message ID</span>
                <div className={styles.fieldContent}>
                  <LinkButton
                    type="external"
                    to={getExplorerUrl('tx', mockTransactionData.messageId, 'Vara')}
                    className={styles.link}>
                    <Address value={mockTransactionData.messageId} />
                  </LinkButton>
                  <CopyButton value={mockTransactionData.messageId} message="Message ID copied to clipboard" />
                </div>
              </div>
            </div>
          </Card>
        </div>
      </div>
    </Container>
  );
}

export { Transaction };
