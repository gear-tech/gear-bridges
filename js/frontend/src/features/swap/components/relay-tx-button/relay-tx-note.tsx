import { useAccount } from '@gear-js/react-hooks';

import { useEthAccount } from '@/hooks';
import { isUndefined } from '@/utils';

import { useIsEthRelayAvailable, useIsVaraRelayAvailable } from '../../hooks';

import styles from './relay-tx-note.module.scss';

type VaraProps = {
  blockNumber: string;
  sender: string;
};

function RelayVaraTxNote({ blockNumber, sender }: VaraProps) {
  const { account } = useAccount();
  const isOwner = account?.decodedAddress === sender;

  const ethAccount = useEthAccount();

  const { data: isAvailable } = useIsVaraRelayAvailable(blockNumber);

  if ((account && !isOwner) || (!account && !ethAccount.address) || isUndefined(isAvailable)) return;

  if (!isAvailable)
    return (
      <div className={styles.text}>
        <p>Transaction is finalizing on the Ethereum chain.</p>
        <p>You can pay the fee now for automatic claim or wait until manual claim becomes available.</p>
      </div>
    );

  return (
    <div className={styles.text}>
      <p>Choose how to claim your tokens: pay a fee to auto-claim, or finalize manually using your wallet.</p>
      {!ethAccount.address && <p>Ethereum wallet connection will be requested.</p>}
    </div>
  );
}

type EthProps = {
  blockNumber: bigint;
  sender: string;
};

function RelayEthTxNote({ blockNumber, sender }: EthProps) {
  const { account } = useAccount();

  const ethAccount = useEthAccount();
  const isOwner = ethAccount.address?.toLowerCase() === sender;

  const { data: isAvailable } = useIsEthRelayAvailable(blockNumber);

  if ((ethAccount.address && !isOwner) || (!ethAccount.address && !account) || isUndefined(isAvailable)) return;

  if (!isAvailable)
    return (
      <div className={styles.text}>
        <p>Transaction is finalizing on the Vara chain.</p>
        <p>You can pay the fee now for automatic claim or wait until manual claim becomes available.</p>
      </div>
    );

  return (
    <div className={styles.text}>
      <p>Choose how to claim your tokens: pay a fee to auto-claim, or finalize manually using your wallet.</p>
      {!account && <p>Vara wallet connection will be requested.</p>}
    </div>
  );
}

const RelayTxNote = {
  Vara: RelayVaraTxNote,
  Eth: RelayEthTxNote,
};

export { RelayTxNote };
