import { getTypedEntries, useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useVaraAccountBalance, useEthAccountBalance, useTokens, useVaraFTBalance } from '@/hooks';
import { isUndefined } from '@/utils';

import { useVaraFTBalances, useEthFTBalances, useBurnVaraTokens } from '../../hooks';
import { BalanceCard } from '../balance-card';

import styles from './tokens-card.module.scss';

function TokensCard() {
  const { account } = useAccount();
  const { addresses, decimals, symbols } = useTokens();
  const burn = useBurnVaraTokens();
  const { data: lockedBalance } = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS);
  const alert = useAlert();

  const isVaraNetwork = Boolean(account);
  const networkIndex = isVaraNetwork ? 0 : 1;

  const nonNativeAddresses = addresses?.filter((pair) => pair[networkIndex] !== WRAPPED_VARA_CONTRACT_ADDRESS);

  const { data: varaFtBalances } = useVaraFTBalances(nonNativeAddresses);
  const { data: ethFtBalances } = useEthFTBalances(nonNativeAddresses);
  const ftBalances = varaFtBalances || ethFtBalances;

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

  const renderBalances = () => {
    if (!ftBalances || !decimals || !symbols)
      return new Array(Object.keys(TOKEN_SVG).length)
        .fill(null)
        .map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return getTypedEntries(ftBalances).map(([address, balance]) => (
      <li key={address}>
        <BalanceCard
          SVG={TOKEN_SVG[address] ?? TokenPlaceholderSVG}
          value={balance}
          decimals={decimals[address] ?? 0}
          symbol={symbols[address] ?? 'Unit'}
        />
      </li>
    ));
  };

  const handleUnlockBalanceClick = () => {
    if (isUndefined(lockedBalance)) throw new Error('Locked balance is not found');

    burn
      .sendTransactionAsync({ args: [lockedBalance] })
      .then(() => alert.success('Tokens unlocked successfully'))
      .catch((error) => alert.error(error instanceof Error ? error.message : String(error)));
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
        {!isUndefined(accountBalance.data) && (
          <li>
            <BalanceCard
              SVG={isVaraNetwork ? VaraSVG : EthSVG}
              value={accountBalance.data}
              decimals={isVaraNetwork ? 12 : 18}
              symbol={isVaraNetwork ? 'VARA' : 'ETH'}
            />
          </li>
        )}

        {renderBalances()}
      </ul>

      {!isUndefined(lockedBalance) && symbols && decimals && (
        <>
          <h4 className={styles.lockedHeading}>Locked Tokens</h4>

          <BalanceCard
            value={lockedBalance}
            SVG={TOKEN_SVG[WRAPPED_VARA_CONTRACT_ADDRESS] ?? TokenPlaceholderSVG}
            decimals={decimals[WRAPPED_VARA_CONTRACT_ADDRESS] ?? 0}
            symbol={symbols[WRAPPED_VARA_CONTRACT_ADDRESS] ?? 'Unit'}
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
