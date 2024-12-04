import { useAccount } from '@gear-js/react-hooks';

import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useVaraFTBalance, useEthAccount, useModal, useTokens } from '@/hooks';

import { MiniWalletModal } from '../mini-wallet-modal';

function MiniWallet() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { decimals } = useTokens();

  const varaLockedBalance = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS, decimals?.[WRAPPED_VARA_CONTRACT_ADDRESS]);

  const [isOpen, open, close] = useModal();

  if (!account && !ethAccount.isConnected) return;

  return (
    <>
      <button type="button" onClick={open}>
        My Tokens
      </button>

      {isOpen && (
        // TODO: remove assertion after @gear-js/vara-ui heading is updated to accept ReactNode.
        // fast fix for now, cuz major font update was made without a fallback,
        <MiniWalletModal lockedBalance={varaLockedBalance} close={close} />
      )}
    </>
  );
}

export { MiniWallet };
