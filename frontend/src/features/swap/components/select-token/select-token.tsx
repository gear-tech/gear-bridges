import { HexString } from '@gear-js/api';
import { Input, Modal } from '@gear-js/vara-ui';
import { useState } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import SearchSVG from '@/assets/search.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FormattedBalance, Skeleton, TokenSVG } from '@/components';
import { useTokens } from '@/context';
import { getAddressToTokenKey } from '@/context/tokens';
import { useEthFTBalances, useVaraFTBalances, useModal, useVaraAccountBalance, useEthAccountBalance } from '@/hooks';
import { cx, isUndefined } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK } from '../../consts';
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
  const { token, network } = useBridgeContext();

  const varaFtBalances = useVaraFTBalances();
  const ethFtBalances = useEthFTBalances();

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();

  const [networkName, setNetworkName] = useState(network.name);
  const isVaraNetwork = networkName === NETWORK.VARA;

  const [searchQuery, setSearchQuery] = useState('');

  const renderTokenBalance = (address: HexString, destAddress: HexString, isNative: boolean) => {
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
        decimals={addressToToken[getAddressToTokenKey(address, destAddress)]?.decimals ?? 0}
        className={styles.balance}
      />
    );
  };

  const filteredTokens = tokens[networkName]?.filter(({ displaySymbol }) => {
    const lowerCaseSymbol = displaySymbol.toLocaleLowerCase();
    const lowerCaseSearchQuery = searchQuery.toLocaleLowerCase();

    return lowerCaseSymbol.includes(lowerCaseSearchQuery);
  });

  const renderTokens = () => {
    if (!filteredTokens) return;

    return filteredTokens.map(({ address, destinationAddress, symbol, displaySymbol, isNative }, index) => {
      const isActive = address === token?.address;
      const networkText = isVaraNetwork ? 'Vara' : 'Ethereum';

      const handleClick = () => {
        token?.set(getAddressToTokenKey(address, destinationAddress));
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
              <TokenSVG symbol={symbol} network={networkName} sizes={[32, 20]} />

              <span className={styles.token}>
                <span className={styles.symbol}>{displaySymbol}</span>
                <span className={styles.network}>{networkText}</span>
              </span>
            </span>

            {renderTokenBalance(address, destinationAddress, isNative)}
          </button>
        </li>
      );
    });
  };

  return (
    <Modal heading="Select Token to Transfer" maxWidth="490px" close={close}>
      <div className={styles.networks}>
        <h4 className={styles.heading}>Select Network</h4>

        <div className={styles.list}>
          <button
            type="button"
            className={cx(styles.network, networkName === NETWORK.VARA && styles.active)}
            disabled={networkName === NETWORK.VARA}
            onClick={() => setNetworkName(NETWORK.VARA)}>
            <VaraSVG />
            <p>Vara</p>
          </button>

          <button
            type="button"
            className={cx(styles.network, networkName === NETWORK.ETH && styles.active)}
            disabled={networkName === NETWORK.ETH}
            onClick={() => setNetworkName(NETWORK.ETH)}>
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
