import { HexString } from '@gear-js/api';
import { Input, Modal } from '@gear-js/vara-ui';
import { useState } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import SearchSVG from '@/assets/search.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FormattedBalance, Skeleton, TokenSVG } from '@/components';
import { useTokens } from '@/context';
import { useEthFTBalances, useVaraFTBalances, useModal, useVaraAccountBalance, useEthAccountBalance } from '@/hooks';
import { cx, isUndefined } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK_INDEX } from '../../consts';
import { useBridgeContext } from '../../context';

import styles from './select-token.module.scss';

type Props = {
  symbol: string;
};

type ModalProps = {
  close: () => void;
};

function SelectTokenModal({ close }: ModalProps) {
  const { tokens, addressToToken } = useTokens();
  const { network, pair } = useBridgeContext();

  const varaFtBalances = useVaraFTBalances();
  const ethFtBalances = useEthFTBalances();

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();

  const [networkIndex, setNetworkIndex] = useState(network.index);
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  // TODO: active filter
  const activeTokens = tokens?.filter(
    ({ isActive, ...token }) => isActive && token.network === (isVaraNetwork ? 'vara' : 'eth'),
  );

  const [searchQuery, setSearchQuery] = useState('');

  const renderTokenBalance = (address: HexString, isNative: boolean) => {
    const ftBalances = isVaraNetwork ? varaFtBalances : ethFtBalances;
    const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

    const ftBalance = { data: ftBalances.data?.[address], isLoading: ftBalances.isLoading };
    const balance = isNative ? accountBalance : ftBalance;

    if (!addressToToken || balance.isLoading) return <Skeleton width="5rem" />;
    if (isUndefined(balance.data)) return;

    return (
      <FormattedBalance
        value={balance.data}
        symbol=""
        decimals={addressToToken[address]?.decimals ?? 0}
        className={styles.balance}
      />
    );
  };

  const filteredTokens = activeTokens?.filter(({ symbol }) => {
    const lowerCaseSymbol = symbol.toLocaleLowerCase();
    const lowerCaseSearchQuery = searchQuery.toLocaleLowerCase();

    return lowerCaseSymbol.includes(lowerCaseSearchQuery);
  });

  const renderTokens = () => {
    if (!filteredTokens) return;

    // TODO: bridge context
    // const selectedTokenAddress = addresses[pair.index][network.index];
    const selectedTokenAddress = '0x00';

    return filteredTokens.map(({ address, symbol, isNative }, index) => {
      const isActive = address === selectedTokenAddress;
      const networkText = isVaraNetwork ? 'Vara' : 'Ethereum';

      const handleClick = () => {
        // TODO: bridge context
        // const indexWithinNonFilteredAddresses = addresses.findIndex((_pair) => _pair[networkIndex] === address);
        const indexWithinNonFilteredAddresses = 0;

        network.setIndex(networkIndex);
        pair.setIndex(indexWithinNonFilteredAddresses);
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
              <TokenSVG symbol={symbol} networkIndex={networkIndex} sizes={[32, 20]} />

              <span className={styles.token}>
                <span className={styles.symbol}>{symbol}</span>
                <span className={styles.network}>{networkText}</span>
              </span>
            </span>

            {renderTokenBalance(address, isNative)}
          </button>
        </li>
      );
    });
  };

  return (
    <Modal heading="Select Token" maxWidth="490px" close={close}>
      <div className={styles.networks}>
        <h4 className={styles.heading}>Network</h4>

        <div className={styles.list}>
          <button
            type="button"
            className={cx(styles.network, networkIndex === NETWORK_INDEX.VARA && styles.active)}
            disabled={networkIndex === NETWORK_INDEX.VARA}
            onClick={() => setNetworkIndex(NETWORK_INDEX.VARA)}>
            <VaraSVG />
            <p>Vara</p>
          </button>

          <button
            type="button"
            className={cx(styles.network, networkIndex === NETWORK_INDEX.ETH && styles.active)}
            disabled={networkIndex === NETWORK_INDEX.ETH}
            onClick={() => setNetworkIndex(NETWORK_INDEX.ETH)}>
            <EthSVG />
            <p>Ethereum</p>
          </button>
        </div>
      </div>

      <Input
        label="Token Name"
        icon={SearchSVG}
        onChange={({ target }) => setSearchQuery(target.value)}
        className={styles.input}
      />

      {filteredTokens?.length ? (
        <ul className={styles.tokens}>{renderTokens()}</ul>
      ) : (
        <p className={styles.notFound}>Tokens with provided name are not found.</p>
      )}
    </Modal>
  );
}

function SelectToken({ symbol }: Props) {
  const [isModalOpen, openModal, closeModal] = useModal();

  return (
    <>
      <button type="button" className={styles.button} onClick={openModal}>
        {symbol}
        <ArrowSVG />
      </button>

      {isModalOpen && <SelectTokenModal close={closeModal} />}
    </>
  );
}

export { SelectToken };
