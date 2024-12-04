import { HexString } from '@gear-js/api';
import { getTypedEntries, useAccount } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { formatUnits } from 'viem';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG } from '@/consts';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/features/swap/consts';
import { useEthAccountBalance, useVaraAccountBalance, useVaraFTBalance } from '@/features/swap/hooks';
import { useEthAccount, useModal, useTokens } from '@/hooks';

import { useVaraFTBalances, useEthFTBalances } from '../../hooks';
import { Balance } from '../balance';

import styles from './mini-wallet.module.scss';

function MiniWallet() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { addresses, decimals, symbols } = useTokens();

  const networkIndex = account ? 0 : 1;
  const nonNativeAddresses = addresses?.filter(
    (pair) => (pair[networkIndex].toString() as HexString) !== WRAPPED_VARA_CONTRACT_ADDRESS,
  );

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const varaLockedBalance = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS, decimals?.[WRAPPED_VARA_CONTRACT_ADDRESS]);

  const { data: varaFtBalances } = useVaraFTBalances(nonNativeAddresses);
  const { data: ethFtBalances } = useEthFTBalances(nonNativeAddresses);

  const [isOpen, open, close] = useModal();

  if (!account && !ethAccount.isConnected) return;

  const ftBalances = varaFtBalances || ethFtBalances;
  const accBalance = account ? varaAccountBalance : ethAccountBalance;

  const renderBalances = () =>
    ftBalances &&
    decimals &&
    symbols &&
    getTypedEntries(ftBalances).map(([address, balance]) => (
      <li key={address} className={styles.card}>
        <Balance
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
    <>
      <button type="button" onClick={open}>
        My Tokens
      </button>

      {isOpen && (
        // TODO: remove assertion after @gear-js/vara-ui heading is updated to accept ReactNode.
        // fast fix for now, cuz major font update was made without a fallback,
        <Modal heading={renderHeading() as unknown as string} close={close}>
          <ul className={styles.list}>
            {accBalance.formattedValue && (
              <li className={styles.card}>
                <Balance
                  SVG={account ? VaraSVG : EthSVG}
                  value={accBalance.formattedValue}
                  symbol={account ? 'VARA' : 'ETH'}
                />
              </li>
            )}

            {renderBalances()}
          </ul>

          {!!varaLockedBalance.value && varaLockedBalance.formattedValue && (
            <div className={styles.locked}>
              <h4 className={styles.heading}>Locked Tokens</h4>

              <div className={styles.card}>
                <Balance
                  value={varaLockedBalance.formattedValue}
                  SVG={account ? VaraSVG : EthSVG}
                  symbol={account ? 'VARA' : 'ETH'}
                />
              </div>
            </div>
          )}
        </Modal>
      )}
    </>
  );
}

export { MiniWallet };
