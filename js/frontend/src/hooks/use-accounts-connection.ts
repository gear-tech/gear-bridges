import { useAccount } from '@gear-js/react-hooks';

import { useEthAccount } from './use-eth-account';

function useAccountsConnection() {
  const { account, isAccountReady } = useAccount();
  const ethAccount = useEthAccount();

  const isVaraAccount = Boolean(account);
  const isEthAccount = Boolean(ethAccount.address);
  const isAnyAccount = isVaraAccount || isEthAccount;

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  const isAnyAccountLoading = !isAccountReady || ethAccount.isReconnecting;

  return {
    isAnyAccount,
    isVaraAccount,
    isEthAccount,
    isAnyAccountLoading,
  };
}

export { useAccountsConnection };
