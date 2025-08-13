import { cx } from '@/utils';

import styles from './truncated-text.module.scss';

type Props = {
  value: string;
  className?: string;
};

function TruncatedText({ value, className, ...props }: Props) {
  return (
    // spreading props for tooltip to work
    <span className={cx(styles.text, className)} {...props}>
      {value}
    </span>
  );
}

export { TruncatedText };
