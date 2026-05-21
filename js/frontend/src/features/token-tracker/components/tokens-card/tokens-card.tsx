import { HexString } from '@gear-js/api';
import { useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { captureException } from '@sentry/react';
import { ReactNode } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { useTokens } from '@/context';
import { GetBalanceButton } from '@/features/faucet';
import { NETWORK } from '@/features/swap/consts';
import {
  useEthAccount,
  useEthAccountBalance,
  useEthFTBalances,
  useVaraAccountBalance,
  useVaraFTBalances,
  useVaraSymbol,
} from '@/hooks';
import { getErrorMessage, isUndefined } from '@/utils';

import { useBurnEthTokens, useBurnVaraTokens } from '../../hooks';
import { BalanceCard } from '../balance-card';

import styles from './tokens-card.module.scss';

type TokensCardProps = {
  network: { name: string; symbol: string | undefined; id: 'vara' | 'eth'; icon: ReactNode; decimals: number };
  useBurn: () => { isPending: boolean; mutateAsync: (value: bigint) => Promise<unknown> };
  useFTBalances: () => { data: Record<string, bigint> | undefined; refetch: () => Promise<unknown> };
  useAccountBalance: () => { data: bigint | undefined };
  renderNativeActions: (address: HexString, onSuccess: () => Promise<unknown>) => ReactNode;
  renderTokenActions?: (address: HexString, symbol: string, onSuccess: () => Promise<unknown>) => ReactNode;
};

function TokensCardComponent({
  network,
  useBurn,
  useFTBalances,
  useAccountBalance,
  renderNativeActions,
  renderTokenActions,
}: TokensCardProps) {
  const alert = useAlert();
  const { tokens, nativeToken } = useTokens();
  const networkTokens = tokens[network.id];
  const networkNativeToken = nativeToken[network.id];

  const burn = useBurn();
  const { data: ftBalances, refetch: refetchBalances } = useFTBalances();
  const accountBalance = useAccountBalance();

  const handleBurnClick = (value: bigint) => {
    burn
      .mutateAsync(value)
      .then(() => {
        alert.success('Tokens converted successfully');

        return refetchBalances();
      })
      .catch((error: Error) => {
        alert.error(getErrorMessage(error));
        captureException(error, { tags: { feature: 'burn-tokens' } });
      });
  };

  const renderBalances = () => {
    if (!networkTokens || !ftBalances || !network.symbol)
      return new Array(5).fill(null).map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return networkTokens.map(({ address, decimals, symbol, isNative }) => {
      const balance = ftBalances[address];

      return (
        <li key={address}>
          <BalanceCard value={balance} decimals={decimals ?? 0} symbol={symbol ?? 'Unit'} network={network.id}>
            {isNative
              ? Boolean(balance) && (
                  <Button
                    text={`Convert To ${network.symbol}`}
                    size="small"
                    onClick={() => handleBurnClick(balance)}
                    isLoading={burn.isPending}
                    className={styles.burnButton}
                  />
                )
              : renderTokenActions?.(address, symbol ?? 'Unit', refetchBalances)}
          </BalanceCard>
        </li>
      );
    });
  };

  return (
    <div className={styles.card}>
      <header className={styles.header}>
        <h2>Tokens</h2>

        <span className={styles.network}>
          {network.icon}
          {network.name}
        </span>
      </header>

      <ul className={styles.list}>
        {!isUndefined(accountBalance.data) && network.symbol && networkNativeToken && (
          <li>
            <BalanceCard
              value={accountBalance.data}
              decimals={network.decimals}
              symbol={network.symbol}
              network={network.id}>
              {renderNativeActions(networkNativeToken.address, refetchBalances)}
            </BalanceCard>
          </li>
        )}

        {renderBalances()}
      </ul>
    </div>
  );
}

function VaraTokensCard() {
  const { account } = useAccount();
  const varaSymbol = useVaraSymbol();

  if (!account) return;

  return (
    <TokensCardComponent
      network={{ name: 'Vara', symbol: varaSymbol, id: NETWORK.VARA, icon: <VaraSVG />, decimals: 12 }}
      useBurn={useBurnVaraTokens}
      useFTBalances={useVaraFTBalances}
      useAccountBalance={useVaraAccountBalance}
      renderNativeActions={() => <GetBalanceButton.VaraAccount />}
    />
  );
}

function EthTokensCard() {
  const ethAccount = useEthAccount();

  if (!ethAccount.address) return;

  return (
    <TokensCardComponent
      network={{ name: 'Ethereum', symbol: 'ETH', id: NETWORK.ETH, icon: <EthSVG />, decimals: 18 }}
      useBurn={useBurnEthTokens}
      useFTBalances={useEthFTBalances}
      useAccountBalance={useEthAccountBalance}
      renderNativeActions={(address, onSuccess) => (
        <GetBalanceButton.EthToken address={address} symbol="ETH" onSuccess={onSuccess} />
      )}
      renderTokenActions={(address, symbol, onSuccess) => (
        <GetBalanceButton.EthToken address={address} symbol={symbol} onSuccess={onSuccess} />
      )}
    />
  );
}

const TokensCard = {
  Vara: VaraTokensCard,
  Eth: EthTokensCard,
};

export { TokensCard };
