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
    return <p className={styles.text}>Waiting for finalization. Manual relay will be available soon.</p>;

  if (!ethAccount.address)
    return (
      <div className={styles.text}>
        <p>Click to complete the transfer and claim tokens.</p>
        <p>Ethereum wallet connection will be prompted.</p>
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
    return <p className={styles.text}>Waiting for finalization. Manual relay will be available soon.</p>;

  if (!account)
    return (
      <div className={styles.text}>
        <p>Click to complete the transfer and claim tokens.</p>
        <p>Vara wallet connection will be prompted.</p>
      </div>
    );
}

const RelayTxNote = {
  Vara: RelayVaraTxNote,
  Eth: RelayEthTxNote,
};

export { RelayTxNote };
