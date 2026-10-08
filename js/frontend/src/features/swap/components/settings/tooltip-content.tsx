import { Link } from 'react-router-dom';

import { ROUTE } from '@/consts';
import { Status } from '@/features/history/types';

import styles from './tooltip-content.module.scss';

function PriorityTooltipContent() {
  return (
    <>
      <p>
        <span className={styles.bold}>Transfer Speed</span> defines how quickly your transfer will be processed:
      </p>

      <ul className={styles.list}>
        <li>
          <span className={styles.bold}>Fast</span> - recommended accelerated processing (~20 minutes) with a higher
          fee.
        </li>

        <li>
          <span className={styles.bold}>Common</span> - standard speed (~1 hour) with a lower fee.
        </li>
      </ul>
    </>
  );
}

function ClaimTypeTooltipContent() {
  return (
    <>
      <p>
        <span className={styles.bold}>Claim Type</span> determines how you receive your tokens:
      </p>

      <ul className={styles.list}>
        <li>
          <span className={styles.bold}>Automatic</span> - recommended. Tokens are delivered to your wallet
          automatically for an additional fee.
        </li>

        <li>
          <span className={styles.bold}>Manual</span> - advanced. After the transfer is completed, you need to claim
          your tokens from the{' '}
          <Link to={`${ROUTE.TRANSACTIONS}?owner=true&status=${Status.AwaitingPayment}`} className={styles.link}>
            Transactions
          </Link>{' '}
          page and pay gas with the destination network&apos;s native token.
        </li>
      </ul>
    </>
  );
}

const TooltipContent = {
  Priority: PriorityTooltipContent,
  ClaimType: ClaimTypeTooltipContent,
};

export { TooltipContent };
