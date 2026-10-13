import { useAccountsConnection } from '@/hooks';

import styles from './relay-tx-note.module.scss';

function RelayVaraTxNote() {
  const { isAnyAccount, isVaraAccount, isEthAccount } = useAccountsConnection();

  if (!isAnyAccount) return;

  return (
    <div className={styles.text}>
      <p>Choose how to claim tokens: pay a fee to auto-claim, or finalize manually using your wallet.</p>
      <p>Manual preparation waits for an authenticated finalized root and that block&apos;s maturity delay.</p>
      {!isVaraAccount && <p>Vara wallet connection will be requested for auto claim.</p>}
      {!isEthAccount && <p>Ethereum wallet connection will be requested for manual claim.</p>}
    </div>
  );
}

function RelayEthTxNote() {
  const { isAnyAccount, isVaraAccount } = useAccountsConnection();

  if (!isAnyAccount) return;

  return (
    <div className={styles.text}>
      <p>Claim tokens: finalize manually using your wallet.</p>
      <p>With an approved profile, preparation waits for an authenticated finalized checkpoint before signing.</p>
      {!isVaraAccount && <p>Vara wallet connection will be requested.</p>}
    </div>
  );
}

const RelayTxNote = {
  Vara: RelayVaraTxNote,
  Eth: RelayEthTxNote,
};

export { RelayTxNote };
