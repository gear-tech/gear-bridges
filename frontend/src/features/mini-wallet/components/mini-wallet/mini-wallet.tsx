import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { Modal } from '@gear-js/vara-ui';
import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { formatUnits } from 'viem';
import { useReadContracts } from 'wagmi';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FUNGIBLE_TOKEN_ABI, TOKEN_SVG, VftProgram } from '@/consts';
import { useEthAccountBalance, useVaraAccountBalance } from '@/features/swap/hooks';
import { useEthAccount, useModal, useTokens } from '@/hooks';
import { SVGComponent } from '@/types';
import { isUndefined } from '@/utils';

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
    queryKey: ['vara-ft-balances', account?.decodedAddress, addresses, symbols, decimals],
    queryFn: getBalances,
    enabled: isApiReady && Boolean(account && addresses && symbols && decimals),
  });
}

function useEthFTBalances() {
  const ethAccount = useEthAccount();
  const { addresses, symbols, decimals } = useTokens();

  const contracts = useMemo(
    () =>
      addresses?.map(([, address]) => ({
        address: address.toString() as HexString,
        abi: FUNGIBLE_TOKEN_ABI,
        functionName: 'balanceOf',
        args: [ethAccount.address],
      })),
    [addresses, ethAccount.address],
  );

  return useReadContracts({
    contracts,
    query: {
      enabled: ethAccount.isConnected,
      select: (data) =>
        addresses &&
        symbols &&
        decimals &&
        data.map(({ result }, index) => {
          const address = addresses?.[index]?.[1].toString() as HexString;

          return {
            address,
            balance: isUndefined(result) ? 0n : BigInt(result),
            symbol: symbols[address],
            decimals: decimals[address],
          };
        }),
    },
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

  const varaAccountBalance = useVaraAccountBalance();
  const ethAccountBalance = useEthAccountBalance();
  const { data: varaFtBalances } = useVaraFTBalances();
  const { data: ethFfBalances } = useEthFTBalances();

  const [isOpen, open, close] = useModal();

  if (!account && !ethAccount.isConnected) return;
  const ftBalances = varaFtBalances || ethFfBalances;
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

  return (
    <>
      <button type="button" onClick={open}>
        My Tokens
      </button>

      {isOpen && (
        <Modal heading="My Tokens" close={close}>
          <Balance SVG={account ? VaraSVG : EthSVG} value={account ? 'Vara' : 'Ethereum'} symbol="" />

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
        </Modal>
      )}
    </>
  );
}

export { MiniWallet };
