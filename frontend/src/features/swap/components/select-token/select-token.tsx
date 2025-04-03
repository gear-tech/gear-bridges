import { HexString } from '@gear-js/api';
import { Input, Modal } from '@gear-js/vara-ui';
import { isUndefined } from '@polkadot/util';
import { useState } from 'react';

import SearchSVG from '@/assets/search.svg?react';
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

  const [searchQuery, setSearchQuery] = useState('');

  const renderTokenBalance = (address: HexString) => {
    const ftBalances = isVaraNetwork ? varaFtBalances : ethFtBalances;
    const isNativeToken = address === WRAPPED_VARA_CONTRACT_ADDRESS;
    const ftBalance = { data: ftBalances.data?.[address], isLoading: ftBalances.isLoading };
    const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance) : ftBalance;

    if (!decimals || balance.isLoading) return <Skeleton width="5rem" />;
    if (isUndefined(balance.data)) return;

    return <FormattedBalance value={balance.data} symbol="" decimals={decimals[address]} className={styles.balance} />;
  };

  const filteredAddresses =
    addresses && symbols
      ? addresses.filter(([varaAddress, ethAddress]) => {
          const address = isVaraNetwork ? varaAddress : ethAddress;
          const lowerCaseSymbol = symbols[address].toLocaleLowerCase();
          const lowerCaseSearchQuery = searchQuery.toLocaleLowerCase();

          return lowerCaseSymbol.includes(lowerCaseSearchQuery);
        })
      : undefined;

  const renderTokens = () => {
    if (!addresses || !filteredAddresses || !symbols) return;

    const selectedTokenAddress = addresses[pairIndex][isVaraNetwork ? 0 : 1];

    return filteredAddresses.map(([varaAddress, ethAddress], index) => {
      const address = isVaraNetwork ? varaAddress : ethAddress;
      const isActive = address === selectedTokenAddress;
      const SVG = TOKEN_SVG[address] ?? TokenPlaceholderSVG;
      const symbol = symbols[address];
      const network = isVaraNetwork ? 'Vara' : 'Ethereum';

      const handleClick = () => {
        const indexWithinNonFilteredAddresses = addresses.findIndex((pair) => pair[isVaraNetwork ? 0 : 1] === address);

        onChange(indexWithinNonFilteredAddresses);
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
                <span className={styles.symbol}>{symbol}</span>
                <span className={styles.network}>{network}</span>
              </span>
            </span>

            {renderTokenBalance(address)}
          </button>
        </li>
      );
    });
  };

  return (
    <Modal heading="Select Token" maxWidth="490px" close={close}>
      <Input
        label="Token Name"
        icon={SearchSVG}
        onChange={({ target }) => setSearchQuery(target.value)}
        className={styles.input}
      />

      {filteredAddresses?.length ? (
        <ul className={styles.tokens}>{renderTokens()}</ul>
      ) : (
        <p className={styles.notFound}>Tokens with provided name are not found.</p>
      )}
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
