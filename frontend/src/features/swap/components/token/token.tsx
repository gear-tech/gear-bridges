import { HexString } from '@gear-js/api';

import { Address, CopyButton, Skeleton, TokenSVG } from '@/components';
import { cx } from '@/utils';

import { SelectToken } from '../select-token';

import styles from './token.module.scss';

type Props = {
  type: 'select' | 'text';
  address: HexString | undefined;
  symbol: string | undefined;
  network: string;
  networkIndex: number;
};

function Token({ type, address, symbol, network, networkIndex }: Props) {
  return (
    <div className={cx(styles.container, styles[type])}>
      <TokenSVG address={address} networkIndex={networkIndex} sizes={[48, 28]} />

      <div className={styles.token}>
        {(!address || !symbol) && <Skeleton width="6rem" />}

        {address && symbol && (
          <>
            {type === 'text' ? <p className={styles.symbol}>{symbol}</p> : <SelectToken symbol={symbol} />}

            <div className={styles.addressContainer}>
              <Address value={address} tooltip={{ side: 'bottom' }} className={styles.address} />

              <CopyButton
                value={address}
                message="Smart contract address copied to clipboard"
                className={styles.copyButton}
              />
            </div>
          </>
        )}

        <p className={styles.network}>{network}</p>
      </div>
    </div>
  );
}

export { Token };
