import { SVGComponent } from '@/types';

import styles from './network-card.module.scss';

type Props = {
  SVG: SVGComponent;
  destination: string;
  name: string;
};

function NetworkCard({ SVG, destination, name }: Props) {
  return (
    <div className={styles.card}>
      {destination}
      <p className={styles.network}>
        <SVG />
        <span className={styles.name}>{name}</span>
      </p>
    </div>
  );
}

export { NetworkCard };
