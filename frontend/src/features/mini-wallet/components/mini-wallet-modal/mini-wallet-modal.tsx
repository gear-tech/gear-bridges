import { HexString } from '@gear-js/api';
import { getTypedEntries, useAccount } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { formatUnits } from 'viem';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG } from '@/consts';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/features/swap/consts';
import { useVaraAccountBalance, useEthAccountBalance } from '@/features/swap/hooks';
import { useTokens } from '@/hooks';

import { useVaraFTBalances, useEthFTBalances } from '../../hooks';
import { Balance } from '../balance';
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

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();

  const accountBalance = account ? varaAccountBalance : ethAccountBalance;
  const ftBalances = varaFtBalances || ethFtBalances;

  const renderBalances = () =>
    ftBalances &&
    decimals &&
    symbols &&
    getTypedEntries(ftBalances).map(([address, balance]) => (
      <li key={address} className={styles.card}>
        <BalanceCard
          SVG={TOKEN_SVG[address] ?? TokenPlaceholderSVG}
          value={formatUnits(balance, decimals[address] ?? 0)}
          symbol={symbols[address] ?? 'Unit'}
        />
      </li>
    ));

  const renderHeading = () => (
    <>
      My Tokens
      <Balance SVG={account ? VaraSVG : EthSVG} value={account ? 'Vara' : 'Ethereum'} symbol="" />
    </>
  );

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

      {!!lockedBalance.value && lockedBalance.formattedValue && (
        <div className={styles.locked}>
          <h4 className={styles.heading}>Locked Tokens</h4>

          <div className={styles.card}>
            <BalanceCard
              value={lockedBalance.formattedValue}
              SVG={account ? VaraSVG : EthSVG}
              symbol={account ? 'VARA' : 'ETH'}
              locked
            />
          </div>
        </div>
      )}
    </Modal>
  );
}

export { MiniWalletModal };
