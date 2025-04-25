import { HexString } from '@gear-js/api';
import { Input, Modal } from '@gear-js/vara-ui';
import { isUndefined } from '@polkadot/util';
import { useState } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import SearchSVG from '@/assets/search.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FormattedBalance, Skeleton, TokenSVG } from '@/components';
import {
  useEthFTBalances,
  useVaraFTBalances,
  useTokens,
  useModal,
  useVaraAccountBalance,
  useEthAccountBalance,
} from '@/hooks';
import { cx, isNativeToken } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { NETWORK_INDEX } from '../../consts';
import { useBridgeContext } from '../../context';

import styles from './select-token.module.scss';

type Props = {
  symbol: string | undefined;
};

type ModalProps = {
  close: () => void;
};

function SelectTokenModal({ close }: ModalProps) {
  const { network, pair } = useBridgeContext();

  const { addresses, symbols, decimals } = useTokens();
  const varaFtBalances = useVaraFTBalances(addresses);
  const ethFtBalances = useEthFTBalances(addresses);
  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();

  const [networkIndex, setNetworkIndex] = useState(network.index);
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  const [searchQuery, setSearchQuery] = useState('');

  const renderTokenBalance = (address: HexString) => {
    const ftBalances = isVaraNetwork ? varaFtBalances : ethFtBalances;
    const accountBalance = isVaraNetwork ? varaAccountBalance : ethAccountBalance;

    const ftBalance = { data: ftBalances.data?.[address], isLoading: ftBalances.isLoading };
    const balance = isNativeToken(address) ? accountBalance : ftBalance;

    if (!decimals || balance.isLoading) return <Skeleton width="5rem" />;
    if (isUndefined(balance.data)) return;

    return <FormattedBalance value={balance.data} symbol="" decimals={decimals[address]} className={styles.balance} />;
  };

  const filteredAddresses =
    addresses && symbols
      ? addresses.filter((addressPair) => {
          const address = addressPair[networkIndex];
          const lowerCaseSymbol = symbols[address].toLocaleLowerCase();
          const lowerCaseSearchQuery = searchQuery.toLocaleLowerCase();

          return lowerCaseSymbol.includes(lowerCaseSearchQuery);
        })
      : undefined;

  const renderTokens = () => {
    if (!addresses || !filteredAddresses || !symbols) return;

    const selectedTokenAddress = addresses[pair.index][network.index];

    return filteredAddresses.map((addressPair, index) => {
      const address = addressPair[networkIndex];
      const isActive = address === selectedTokenAddress;
      const symbol = symbols[address];
      const networkText = isVaraNetwork ? 'Vara' : 'Ethereum';

      const handleClick = () => {
        const indexWithinNonFilteredAddresses = addresses.findIndex((_pair) => _pair[networkIndex] === address);

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
              <TokenSVG address={address} networkIndex={networkIndex} sizes={[32, 20]} />

              <span className={styles.token}>
                <span className={styles.symbol}>{symbol}</span>
                <span className={styles.network}>{networkText}</span>
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

      {filteredAddresses?.length ? (
        <ul className={styles.tokens}>{renderTokens()}</ul>
      ) : (
        <p className={styles.notFound}>Tokens with provided name are not found.</p>
      )}
    </Modal>
  );
}

function SelectToken({ symbol }: Props) {
  const [isModalOpen, openModal, closeModal] = useModal();

  if (!symbol) return <Skeleton width="6rem" height="24px" />;

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
