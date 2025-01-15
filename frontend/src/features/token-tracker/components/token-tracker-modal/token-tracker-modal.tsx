import { getTypedEntries, useAccount, useAlert } from '@gear-js/react-hooks';
import { Modal, Button } from '@gear-js/vara-ui';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useBridge } from '@/contexts';
import { useVaraAccountBalance, useEthAccountBalance, useTokens } from '@/hooks';
import { isUndefined } from '@/utils';

import { useVaraFTBalances, useEthFTBalances, useBurnVaraTokens } from '../../hooks';
import { BalanceCard } from '../card';

import styles from './token-tracker-modal.module.scss';

type Props = {
  lockedBalance: bigint | undefined;
  close: () => void;
};

function TokenTrackerModal({ lockedBalance, close }: Props) {
  const { account } = useAccount();
  const { addresses, decimals, symbols } = useTokens();
  const { setPairIndex } = useBridge();
  const burn = useBurnVaraTokens();
  const alert = useAlert();

  const isVaraNetwork = Boolean(account);
  const networkIndex = isVaraNetwork ? 0 : 1;

  const nativePairIndex = addresses?.findIndex((pair) => pair[networkIndex] === WRAPPED_VARA_CONTRACT_ADDRESS);
  const nonNativeAddresses = addresses?.filter((pair) => pair[networkIndex] !== WRAPPED_VARA_CONTRACT_ADDRESS);

  const { data: varaFtBalances } = useVaraFTBalances(nonNativeAddresses);
  const { data: ethFtBalances } = useEthFTBalances(nonNativeAddresses);
  const ftBalances = varaFtBalances || ethFtBalances;

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

  const renderNativeToken = () => (
    <span className={styles.network}>
      {isVaraNetwork ? <VaraSVG /> : <EthSVG />}
      {isVaraNetwork ? 'Vara' : 'Ethereum'}
    </span>
  );

  const handleTransferClick = (index: number) => {
    setPairIndex(index);
    close();
  };

  const renderBalances = () => {
    if (!ftBalances || !decimals || !symbols)
      return new Array(Object.keys(TOKEN_SVG).length)
        .fill(null)
        .map((_item, index) => <BalanceCard.Skeleton key={index} />);

    return getTypedEntries(ftBalances).map(([address, { balance, pairIndex }]) => (
      <li key={address}>
        <BalanceCard
          SVG={TOKEN_SVG[address] ?? TokenPlaceholderSVG}
          value={balance}
          decimals={decimals[address] ?? 0}
          symbol={symbols[address] ?? 'Unit'}>
          <Button text="Transfer" color="grey" size="small" onClick={() => handleTransferClick(pairIndex)} />
        </BalanceCard>
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
    <Modal heading="My Tokens" headerAddon={renderNativeToken()} close={close} maxWidth="large">
      <ul className={styles.list}>
        {!isUndefined(accountBalance.data) && (
          <li>
            <BalanceCard
              SVG={isVaraNetwork ? VaraSVG : EthSVG}
              value={accountBalance.data}
              decimals={isVaraNetwork ? 12 : 18}
              symbol={isVaraNetwork ? 'VARA' : 'ETH'}>
              {!isUndefined(nativePairIndex) && nativePairIndex !== -1 && (
                <Button
                  text="Transfer"
                  color="grey"
                  size="small"
                  onClick={() => handleTransferClick(nativePairIndex)}
                />
              )}
            </BalanceCard>
          </li>
        )}

        {renderBalances()}
      </ul>

      {!isUndefined(lockedBalance) && symbols && decimals && (
        <>
          <h4 className={styles.heading}>Locked Tokens</h4>

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
    </Modal>
  );
}

export { TokenTrackerModal };
