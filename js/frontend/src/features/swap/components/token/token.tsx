import { HexString } from '@gear-js/api';

import { Address, CopyButton, Skeleton, TokenSVG } from '@/components';
import { cx } from '@/utils';

import { SelectToken } from '../select-token';

import styles from './token.module.scss';

type Props = {
  type: 'select' | 'text';
  address: HexString | undefined;
  symbol: string | undefined;
  network: 'vara' | 'eth';
  networkText: string;
};

function Token({ type, address, symbol, network, networkText }: Props) {
  return (
    <div className={cx(styles.container, styles[type])}>
      <TokenSVG symbol={symbol} network={network} />

      <div className={styles.token}>
        <div className={styles.info}>
          {(!address || !symbol) && (
            <>
              <Skeleton width="6rem" height="24px" />
              <Skeleton width="4rem" height="12px" />
            </>
          )}

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
        </div>

        <p className={styles.network}>{networkText}</p>
      </div>
    </div>
  );
}

export { Token };
