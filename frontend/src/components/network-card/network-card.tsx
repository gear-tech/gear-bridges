import { SVGComponent } from '@/types';

import styles from './network-card.module.scss';

type Props = {
  SVG: SVGComponent;
  name: string;
};

function NetworkCard({ SVG, name }: Props) {
  return (
    <div className={styles.card}>
      <SVG />

      <p>
        <span className={styles.name}>{name}</span>
        <span className={styles.text}>Network</span>
      </p>
    </div>
  );
}

export { NetworkCard };
