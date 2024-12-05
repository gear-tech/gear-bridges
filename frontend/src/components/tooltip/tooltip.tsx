import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import QuestionSVG from './question.svg?react';
import styles from './tooltip.module.scss';

type Props = {
  text: string;
  position?: 'top' | 'bottom-end';
  SVG?: SVGComponent;
};

function Tooltip({ text, position = 'top', SVG = QuestionSVG }: Props) {
  return (
    <div className={styles.container}>
      <SVG />

      <div className={cx(styles.tooltip, styles[position])}>
        <p className={styles.text}>{text}</p>
      </div>
    </div>
  );
}

export { Tooltip };
