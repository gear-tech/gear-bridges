import { useAccount } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { formatUnits } from 'viem';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG } from '@/consts';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/features/swap/consts';
import { useEthAccountBalance, useVaraAccountBalance } from '@/features/swap/hooks';
import { useEthAccount, useModal } from '@/hooks';

import { useVaraFTBalances, useEthFTBalances } from '../../hooks';
import { Balance } from '../balance';

import styles from './mini-wallet.module.scss';

function MiniWallet() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const { data: varaFtBalances } = useVaraFTBalances();
  const { data: ethFtBalances } = useEthFTBalances();

  const [isOpen, open, close] = useModal();

  if (!account && !ethAccount.isConnected) return;

  const ftBalances = (varaFtBalances || ethFtBalances)?.filter(
    ({ address }) => address !== WRAPPED_VARA_CONTRACT_ADDRESS,
  );

  const lockedBalance = (varaFtBalances || ethFtBalances)?.filter(
    ({ address }) => address === WRAPPED_VARA_CONTRACT_ADDRESS,
  )[0];

  const accBalance = account ? varaAccountBalance : ethAccountBalance;

  const renderBalances = () =>
    ftBalances?.map(({ address, balance, decimals, symbol }) => (
      <li key={address} className={styles.card}>
        <Balance
          SVG={TOKEN_SVG[address] ?? TokenPlaceholderSVG}
          value={formatUnits(balance, decimals)}
          symbol={symbol}
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

          {!!lockedBalance?.balance && (
            <div className={styles.locked}>
              <h4 className={styles.heading}>Locked Tokens</h4>

              <div className={styles.card}>
                <Balance
                  value={formatUnits(lockedBalance.balance, lockedBalance.decimals)}
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
