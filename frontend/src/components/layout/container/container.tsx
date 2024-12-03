import { useAccount } from '@gear-js/react-hooks';
import { CSSProperties, ReactNode } from 'react';

import { useEthAccount } from '@/hooks';
import { cx } from '@/utils';

import styles from './container.module.scss';

type Props = {
  children: ReactNode;
  maxWidth?: 'xl' | 'md';
  className?: string;
};

const WAVES_COUNT = 5;

function Container({ children, maxWidth = 'xl', className }: Props) {
  return <div className={cx(styles.container, styles[maxWidth], className)}>{children}</div>;
}

function Live({ children, ...props }: Props) {
  const ethAccount = useEthAccount();
  const { account } = useAccount();

  const isAccount = account || ethAccount.isConnected;

  const renderWaves = () =>
    new Array(WAVES_COUNT)
      .fill(null)
      .map((_, index) => (
        <span
          key={index}
          className={cx(styles.wave, isAccount && styles.active)}
          style={{ '--i': index } as CSSProperties}
        />
      ));

  return (
    <Container {...props}>
      {children}
      {renderWaves()}
    </Container>
  );
}

Container.Live = Live;

export { Container };
