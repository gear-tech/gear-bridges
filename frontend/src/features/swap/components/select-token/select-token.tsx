import { Modal } from '@gear-js/vara-ui';
import { isUndefined } from '@polkadot/util';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import { FormattedBalance, Skeleton } from '@/components';
import { TOKEN_SVG, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useEthFTBalances, useVaraFTBalances, useModal, useTokens } from '@/hooks';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { UseAccountBalance } from '../../types';
import { getMergedBalance } from '../../utils';

import styles from './select-token.module.scss';

type Props = {
  pairIndex: number;
  isVaraNetwork: boolean;
  symbol: string | undefined;
  accountBalance: ReturnType<UseAccountBalance>;
  onChange: (value: number) => void;
};

type ModalProps = Pick<Props, 'isVaraNetwork' | 'pairIndex' | 'accountBalance' | 'onChange'> & {
  close: () => void;
};

function SelectTokenModal({ isVaraNetwork, pairIndex, accountBalance, onChange, close }: ModalProps) {
  const { addresses, symbols, decimals } = useTokens();

  const varaFtBalances = useVaraFTBalances(addresses);
  const ethFtBalances = useEthFTBalances(addresses);
  const ftBalances = isVaraNetwork ? varaFtBalances : ethFtBalances;

  const renderTokens = () => {
    if (!addresses || !symbols) return;

    return addresses.map(([varaAddress, ethAddress], index) => {
      const address = isVaraNetwork ? varaAddress : ethAddress;

      const isActive = index === pairIndex;
      const SVG = TOKEN_SVG[address] ?? TokenPlaceholderSVG;

      const isNativeToken = address === WRAPPED_VARA_CONTRACT_ADDRESS;
      const ftBalance = { data: ftBalances.data?.[address], isLoading: ftBalances.isLoading };
      const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance) : ftBalance;

      const handleClick = () => {
        onChange(index);
        close();
      };

      return (
        <li key={index}>
          <button
            type="button"
            className={cx(styles.tokenButton, isActive && styles.active)}
            onClick={handleClick}
            disabled={isActive}>
            <span className={styles.wallet}>
              <SVG />

              <span className={styles.token}>
                <span className={styles.symbol}>{symbols[address]}</span>
                <span className={styles.network}>{isVaraNetwork ? 'Vara' : 'Ethereum'}</span>
              </span>
            </span>

            {!decimals || balance.isLoading ? (
              <Skeleton width="5rem" />
            ) : (
              !isUndefined(balance.data) && (
                <FormattedBalance
                  value={balance.data}
                  symbol=""
                  decimals={decimals[address]}
                  className={styles.balance}
                />
              )
            )}
          </button>
        </li>
      );
    });
  };

  return (
    <Modal heading="Select Token" maxWidth="490px" close={close}>
      <ul className={styles.tokens}>{renderTokens()}</ul>
    </Modal>
  );
}

function SelectToken({ pairIndex, isVaraNetwork, symbol, accountBalance, onChange }: Props) {
  const [isModalOpen, openModal, closeModal] = useModal();

  if (!symbol) return <Skeleton width="6rem" height="24px" />;

  return (
    <>
      <button type="button" className={styles.button} onClick={openModal}>
        {symbol}
        <ArrowSVG />
      </button>

      {isModalOpen && (
        <SelectTokenModal
          isVaraNetwork={isVaraNetwork}
          pairIndex={pairIndex}
          accountBalance={accountBalance}
          onChange={onChange}
          close={closeModal}
        />
      )}
    </>
  );
}

export { SelectToken };
