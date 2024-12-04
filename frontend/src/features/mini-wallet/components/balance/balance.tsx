import { SVGComponent } from '@/types';

import styles from './balance.module.scss';

type Props = {
  SVG: SVGComponent;
  value: string;
  symbol: string;
};

function Balance({ value, SVG, symbol }: Props) {
  return (
    <span className={styles.balance}>
      <SVG />
      {value} {symbol}
    </span>
  );
}

export { Balance };
