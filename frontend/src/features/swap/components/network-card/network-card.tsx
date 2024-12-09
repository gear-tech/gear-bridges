import { SVGComponent } from '@/types';

import styles from './network-card.module.scss';

type Props = {
  SVG: SVGComponent;
  name: string;
};

function NetworkCard({ SVG, name }: Props) {
  return (
    <div className={styles.card}>
      From
      <p className={styles.network}>
        <SVG />
        <span className={styles.name}>{name}</span>
      </p>
    </div>
  );
}

export { NetworkCard };
