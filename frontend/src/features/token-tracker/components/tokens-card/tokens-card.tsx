import { useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { useTokens } from '@/context';
import { GetBalanceButton } from '@/features/faucet';
import { NETWORK } from '@/features/swap/consts';
import {
  useVaraAccountBalance,
  useEthAccountBalance,
  useVaraFTBalances,
  useEthFTBalances,
  useVaraSymbol,
} from '@/hooks';
import { getErrorMessage, isUndefined } from '@/utils';

import { useBurnEthTokens, useBurnVaraTokens } from '../../hooks';
import { BalanceCard } from '../balance-card';

import styles from './tokens-card.module.scss';

function TokensCard() {
  const { account } = useAccount();
  const isVaraNetwork = Boolean(account);
  const network = isVaraNetwork ? NETWORK.VARA : NETWORK.ETH;

  const varaSymbol = useVaraSymbol();
  const networkSymbol = isVaraNetwork ? varaSymbol : 'ETH';
  const alert = useAlert();

  const { tokens, nativeToken: _nativeToken } = useTokens();
  const networkTokens = tokens[network];
  const nativeToken = _nativeToken?.[network];

  const burnVara = useBurnVaraTokens();
  const burnEth = useBurnEthTokens();
  const burn = account ? burnVara : burnEth;

  const { data: varaFtBalances, refetch: refetchVaraBalances } = useVaraFTBalances();
  const { data: ethFtBalances, refetch: refetchEthBalances } = useEthFTBalances();
  const ftBalances = varaFtBalances || ethFtBalances;

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

  const handleBurnClick = (value: bigint) => {
    const sendTx = account ? () => burnVara.sendTransactionAsync({ args: [value] }) : () => burnEth.mutateAsync(value);
    const refetchBalances = account ? refetchVaraBalances : refetchEthBalances;

    sendTx()
      .then(async () => {
        alert.success('Tokens converted successfully');

        return refetchBalances();
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));
  };

  const renderBalances = () => {
    if (!networkTokens || !ftBalances || !networkSymbol)
      return new Array(4).fill(null).map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return networkTokens.map(({ address, decimals, symbol, isNative }) => {
      const balance = ftBalances[address];

      return (
        <li key={address}>
          <BalanceCard value={balance} decimals={decimals ?? 0} symbol={symbol ?? 'Unit'} network={network}>
            {isNative ? (
              Boolean(balance) && (
                <Button
                  text={`Convert To ${networkSymbol}`}
                  size="small"
                  onClick={() => handleBurnClick(balance)}
                  isLoading={burn.isPending}
                  className={styles.burnButton}
                />
              )
            ) : (
              <GetBalanceButton.EthToken address={address} symbol={symbol ?? 'Unit'} onSuccess={refetchEthBalances} />
            )}
          </BalanceCard>
        </li>
      );
    });
  };

  return (
    <div className={styles.card}>
      <header className={styles.header}>
        <h2>My Tokens</h2>

        <span className={styles.network}>
          {isVaraNetwork ? <VaraSVG /> : <EthSVG />}
          {isVaraNetwork ? 'Vara' : 'Ethereum'}
        </span>
      </header>

      <ul className={styles.list}>
        {!isUndefined(accountBalance.data) && networkSymbol && nativeToken && (
          <li>
            <BalanceCard
              value={accountBalance.data}
              decimals={isVaraNetwork ? 12 : 18}
              symbol={networkSymbol}
              network={network}>
              {isVaraNetwork ? (
                <GetBalanceButton.VaraAccount />
              ) : (
                <GetBalanceButton.EthToken address={nativeToken.address} symbol="ETH" onSuccess={refetchEthBalances} />
              )}
            </BalanceCard>
          </li>
        )}

        {renderBalances()}
      </ul>
    </div>
  );
}

export { TokensCard };
