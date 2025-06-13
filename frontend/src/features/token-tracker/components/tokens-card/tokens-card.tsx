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
  const alert = useAlert();

  const { tokens, nativeToken: _nativeToken } = useTokens();
  const nativeToken = _nativeToken?.[network];
  const nonNativeTokens = tokens[network]?.filter(({ isNative }) => !isNative);

  const burnVara = useBurnVaraTokens();
  const burnEth = useBurnEthTokens();
  const burn = account ? burnVara : burnEth;

  const { data: varaFtBalances, refetch: refetchVaraBalances } = useVaraFTBalances();
  const { data: ethFtBalances, refetch: refetchEthBalances } = useEthFTBalances();

  const ftBalances = varaFtBalances || ethFtBalances;
  const lockedBalance = nativeToken?.address ? ftBalances?.[nativeToken.address] : undefined;

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

  const renderBalances = () => {
    if (!nonNativeTokens || !ftBalances)
      return new Array(4).fill(null).map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return nonNativeTokens.map(({ address, decimals, symbol }) => {
      const balance = ftBalances[address];

      return (
        <li key={address}>
          <BalanceCard value={balance} decimals={decimals ?? 0} symbol={symbol ?? 'Unit'} network={network}>
            <GetBalanceButton.EthToken address={address} symbol={symbol ?? 'Unit'} onSuccess={refetchEthBalances} />
          </BalanceCard>
        </li>
      );
    });
  };

  const handleUnlockBalanceClick = () => {
    if (isUndefined(lockedBalance)) throw new Error('Locked balance is not found');

    const sendTx = account
      ? () => burnVara.sendTransactionAsync({ args: [lockedBalance] })
      : () => burnEth.mutateAsync(lockedBalance);

    const refetchBalances = account ? refetchVaraBalances : refetchEthBalances;

    sendTx()
      .then(async () => {
        alert.success('Tokens unlocked successfully');

        return refetchBalances();
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));
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
        {!isUndefined(accountBalance.data) && varaSymbol && nativeToken && (
          <li>
            <BalanceCard
              value={accountBalance.data}
              decimals={isVaraNetwork ? 12 : 18}
              symbol={isVaraNetwork ? varaSymbol : 'ETH'}
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

      {!isUndefined(lockedBalance) && nativeToken && (
        <>
          <h4 className={styles.lockedHeading}>Locked Tokens</h4>

          <BalanceCard
            value={lockedBalance}
            decimals={nativeToken.decimals ?? 0}
            symbol={nativeToken.symbol ?? 'Unit'}
            network={network}
            locked>
            {Boolean(lockedBalance) && (
              <Button text="Unlock" size="small" onClick={handleUnlockBalanceClick} isLoading={burn.isPending} />
            )}
          </BalanceCard>
        </>
      )}
    </div>
  );
}

export { TokensCard };
