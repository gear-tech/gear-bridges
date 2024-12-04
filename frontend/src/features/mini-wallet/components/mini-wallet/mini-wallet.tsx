import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { useQuery } from '@tanstack/react-query';
import { formatUnits } from 'viem';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { TOKEN_SVG, VftProgram } from '@/consts';
import { useEthAccount, useModal, useTokens } from '@/hooks';
import { SVGComponent } from '@/types';

import styles from './mini-wallet.module.scss';

function useVaraFTBalances() {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const { addresses, decimals, symbols } = useTokens();

  const getBalances = async () => {
    if (!api) throw new Error('API not initialized');
    if (!account) throw new Error('Account not found');
    if (!addresses || !symbols || !decimals) throw new Error('Fungible tokens are not found');

    const balancePromises = addresses.map(async ([_address]) => {
      const address = _address.toString() as HexString;

      return {
        address,
        balance: await new VftProgram(api, address).vft.balanceOf(account.decodedAddress),
        symbol: symbols[address],
        decimals: decimals[address],
      };
    });

    return Promise.all(balancePromises);
  };

  return useQuery({
    queryKey: ['vara-ft-balances'],
    queryFn: getBalances,
    enabled: isApiReady && Boolean(account && addresses),
  });
}

function Balance({ value, SVG, symbol }: { value: string; symbol: string; SVG: SVGComponent }) {
  return (
    <span className={styles.balance}>
      <SVG />
      {value} {symbol}
    </span>
  );
}

function MiniWallet() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { data: ftBalances } = useVaraFTBalances();

  const [isOpen, open, close] = useModal();

  if (!account && !ethAccount.isConnected) return;

  const renderBalances = () =>
    ftBalances?.map(({ address, balance, decimals, symbol }) => (
      <li key={address}>
        <Balance
          SVG={TOKEN_SVG[address] ?? TokenPlaceholderSVG}
          value={formatUnits(balance, decimals)}
          symbol={symbol}
        />
      </li>
    ));

  return (
    <>
      <button type="button" onClick={open}>
        My Tokens
      </button>

      {isOpen && (
        <Modal heading="My Tokens" close={close}>
          <Balance SVG={account ? VaraSVG : EthSVG} value={account ? 'Vara' : 'Ethereum'} symbol="" />

          {ftBalances?.length && <ul className={styles.list}>{renderBalances()}</ul>}
        </Modal>
      )}
    </>
  );
}

export { MiniWallet };
