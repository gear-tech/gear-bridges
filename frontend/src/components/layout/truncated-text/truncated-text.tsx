import { cx } from '@/utils';

import styles from './truncated-text.module.scss';

type Props = {
  value: string;
  className?: string;
};

function TruncatedText({ value, className }: Props) {
  return <span className={cx(styles.text, className)}>{value}</span>;
}

export { TruncatedText };
