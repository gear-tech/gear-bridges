import { HexString } from '@gear-js/api';
import { getTypedEntries, useAccount } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { formatUnits } from 'viem';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useVaraAccountBalance, useEthAccountBalance, useTokens } from '@/hooks';

import { useVaraFTBalances, useEthFTBalances } from '../../hooks';
import { BalanceCard } from '../card';

import styles from './mini-wallet-modal.module.scss';

type Props = {
  lockedBalance: { value: bigint | undefined; formattedValue: string | undefined };
  close: () => void;
};

function MiniWalletModal({ lockedBalance, close }: Props) {
  const { account } = useAccount();
  const { addresses, decimals, symbols } = useTokens();

  const networkIndex = account ? 0 : 1;
  const nonNativeAddresses = addresses?.filter(
    (pair) => (pair[networkIndex].toString() as HexString) !== WRAPPED_VARA_CONTRACT_ADDRESS,
  );

  const { data: varaFtBalances } = useVaraFTBalances(nonNativeAddresses);
  const { data: ethFtBalances } = useEthFTBalances(nonNativeAddresses);
  const ftBalances = varaFtBalances || ethFtBalances;

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const accountBalance = account ? varaAccountBalance : ethAccountBalance;

  const renderHeading = () => (
    <>
      My Tokens
      <span className={styles.network}>
        {account ? <VaraSVG /> : <EthSVG />}
        {account ? 'Vara' : 'Ethereum'}
      </span>
    </>
  );

  const renderBalances = () => {
    if (!ftBalances || !decimals || !symbols)
      return new Array(Object.keys(TOKEN_SVG).length)
        .fill(null)
        .map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return getTypedEntries(ftBalances).map(([address, balance]) => (
      <li key={address} className={styles.card}>
        <BalanceCard
          SVG={TOKEN_SVG[address] ?? TokenPlaceholderSVG}
          value={formatUnits(balance, decimals[address] ?? 0)}
          symbol={symbols[address] ?? 'Unit'}
        />
      </li>
    ));
  };

  return (
    // TODO: remove assertion after @gear-js/vara-ui heading is updated to accept ReactNode.
    // fast fix for now, cuz major font update was made without a fallback,
    <Modal heading={renderHeading() as unknown as string} close={close}>
      <ul className={styles.list}>
        {accountBalance.formattedValue && (
          <li>
            <BalanceCard
              SVG={account ? VaraSVG : EthSVG}
              value={accountBalance.formattedValue}
              symbol={account ? 'VARA' : 'ETH'}
            />
          </li>
        )}

        {renderBalances()}
      </ul>

      {lockedBalance.formattedValue && symbols && (
        <div className={styles.locked}>
          <h4 className={styles.heading}>Locked Tokens</h4>

          <div className={styles.card}>
            <BalanceCard
              value={lockedBalance.formattedValue}
              SVG={TOKEN_SVG[WRAPPED_VARA_CONTRACT_ADDRESS] ?? TokenPlaceholderSVG}
              symbol={symbols[WRAPPED_VARA_CONTRACT_ADDRESS] ?? 'Unit'}
              locked
            />
          </div>
        </div>
      )}
    </Modal>
  );
}

export { MiniWalletModal };
