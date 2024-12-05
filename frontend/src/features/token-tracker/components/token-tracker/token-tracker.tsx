import { useAccount } from '@gear-js/react-hooks';

import { Tooltip } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useVaraFTBalance, useEthAccount, useModal, useTokens } from '@/hooks';

import WarningSVG from '../../assets/warning.svg?react';
import { TokenTrackerModal } from '../token-tracker-modal';

import styles from './token-tracker.module.scss';

function TokenTracker() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { decimals } = useTokens();

  const varaLockedBalance = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS, decimals?.[WRAPPED_VARA_CONTRACT_ADDRESS]);

  const [isOpen, open, close] = useModal();

  if (!account && !ethAccount.isConnected) return;

  return (
    <>
      <div className={styles.container}>
        <button type="button" onClick={open}>
          My Tokens
        </button>

        {Boolean(varaLockedBalance.value) && (
          <Tooltip SVG={WarningSVG} text="You have tokens available to unlock" position="bottom-end" />
        )}
      </div>

      {isOpen && <TokenTrackerModal lockedBalance={varaLockedBalance} close={close} />}
    </>
  );
}

export { TokenTracker };
