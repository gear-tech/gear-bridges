import { getTypedEntries, useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { GetBalanceButton } from '@/features/faucet';
import {
  useVaraAccountBalance,
  useEthAccountBalance,
  useTokens,
  useVaraFTBalance,
  useVaraFTBalances,
  useEthFTBalances,
  useEthFTBalance,
  useVaraSymbol,
} from '@/hooks';
import { getErrorMessage, isUndefined } from '@/utils';

import { useBurnEthTokens, useBurnVaraTokens } from '../../hooks';
import { BalanceCard } from '../balance-card';

import styles from './tokens-card.module.scss';

function TokensCard() {
  const { account } = useAccount();
  const varaSymbol = useVaraSymbol();
  const { addresses, decimals, symbols, wrappedVaraAddress, wrappedEthAddress } = useTokens();
  const alert = useAlert();

  const burnVara = useBurnVaraTokens();
  const burnEth = useBurnEthTokens();
  const burn = account ? burnVara : burnEth;

  // TODO: is there any reason not to fetch it from useFTBalances hook?
  const { data: varaLockedBalance, refetch: refetchLockedVara } = useVaraFTBalance(wrappedVaraAddress);
  const { data: ethLockedBalance, refetch: refetchLockedEth } = useEthFTBalance(wrappedEthAddress);
  const lockedBalance = account ? varaLockedBalance : ethLockedBalance;

  const isVaraNetwork = Boolean(account);
  const networkIndex = isVaraNetwork ? 0 : 1;

  const nonNativeAddresses = addresses?.filter(
    (pair) => pair[networkIndex] !== wrappedVaraAddress && pair[networkIndex] !== wrappedEthAddress,
  );

  const { data: varaFtBalances, refetch: refetchVaraBalances } = useVaraFTBalances(nonNativeAddresses);
  const { data: ethFtBalances, refetch: refetchEthBalances } = useEthFTBalances(nonNativeAddresses);
  const ftBalances = varaFtBalances || ethFtBalances;

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

  const renderBalances = () => {
    if (!ftBalances || !decimals || !symbols)
      return new Array(8).fill(null).map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return getTypedEntries(ftBalances).map(([address, balance]) => (
      <li key={address}>
        <BalanceCard
          value={balance}
          decimals={decimals[address] ?? 0}
          symbol={symbols[address] ?? 'Unit'}
          networkIndex={networkIndex}>
          <GetBalanceButton.EthToken
            address={address}
            symbol={symbols[address] ?? 'Unit'}
            onSuccess={refetchEthBalances}
          />
        </BalanceCard>
      </li>
    ));
  };

  const handleUnlockBalanceClick = () => {
    if (isUndefined(lockedBalance)) throw new Error('Locked balance is not found');

    const sendTx = account
      ? () => burnVara.sendTransactionAsync({ args: [lockedBalance] })
      : () => burnEth.mutateAsync(lockedBalance);

    const refetchBalances = account ? refetchVaraBalances : refetchEthBalances;
    const refetchLockedBalance = account ? refetchLockedVara : refetchLockedEth;

    sendTx()
      .then(async () => {
        alert.success('Tokens unlocked successfully');

        await refetchLockedBalance();
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
        {!isUndefined(accountBalance.data) && varaSymbol && wrappedEthAddress && (
          <li>
            <BalanceCard
              value={accountBalance.data}
              decimals={isVaraNetwork ? 12 : 18}
              symbol={isVaraNetwork ? varaSymbol : 'ETH'}
              networkIndex={networkIndex}>
              {isVaraNetwork ? (
                <GetBalanceButton.VaraAccount />
              ) : (
                <GetBalanceButton.EthToken address={wrappedEthAddress} symbol="ETH" onSuccess={refetchEthBalances} />
              )}
            </BalanceCard>
          </li>
        )}

        {renderBalances()}
      </ul>

      {!isUndefined(lockedBalance) && symbols && decimals && wrappedVaraAddress && wrappedEthAddress && (
        <>
          <h4 className={styles.lockedHeading}>Locked Tokens</h4>

          <BalanceCard
            value={lockedBalance}
            decimals={decimals[account ? wrappedVaraAddress : wrappedEthAddress] ?? 0}
            symbol={symbols[account ? wrappedVaraAddress : wrappedEthAddress] ?? 'Unit'}
            networkIndex={networkIndex}
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
