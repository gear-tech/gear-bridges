import { useAccount, useAccountDeriveBalancesAll, useApi, useBalanceFormat } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import VaraSVG from '@/assets/vara.svg?react';
import { useModal } from '@/hooks';

import { AccountButton } from '../account-button';
import { WalletModal } from '../wallet-modal';

import styles from './wallet.module.scss';

function Wallet() {
  const { isApiReady } = useApi();
  const { account, isAccountReady } = useAccount();
  const balances = useAccountDeriveBalancesAll();
  const [isModalOpen, openModal, closeModal] = useModal();

  const { getFormattedBalance } = useBalanceFormat();
  const balance = isApiReady && balances ? getFormattedBalance(balances.freeBalance) : null;

  return isAccountReady ? (
    <>
      {account ? (
        <div className={styles.wallet}>
          {balances && (
            <span className={styles.balance}>
              <VaraSVG />

              {balance && (
                <span className={styles.text}>
                  <span className={styles.value}>{balance.value}</span>
                  <span className={styles.unit}>{balance.unit}</span>
                </span>
              )}
            </span>
          )}

          <AccountButton color="dark" address={account.address} name={account.meta.name} onClick={openModal} />
        </div>
      ) : (
        <Button text="Connect Wallet" onClick={openModal} />
      )}

      {isModalOpen && <WalletModal close={closeModal} />}
    </>
  ) : null;
}

export { Wallet };
