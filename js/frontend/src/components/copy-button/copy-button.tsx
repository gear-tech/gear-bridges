import { useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import { SVGComponent } from '@/types';
import { cx, logger } from '@/utils';

import styles from './copy-button.module.scss';
import CopySVG from './copy.svg?react';

type Props = {
  value: string;
  message?: string;
  SVG?: SVGComponent;
  className?: string;
  onCopy?: () => void;
};

function CopyButton({ value, message = 'Copied', SVG = CopySVG, className, onCopy = () => {}, ...props }: Props) {
  const alert = useAlert();

  const onSuccess = () => {
    alert.success(message);
    onCopy();
  };

  const onError = (error: unknown) => {
    const errorMessage = error instanceof Error ? error.message : 'Unexpected error copying to clipboard';

    alert.error(errorMessage);
    logger.error('Copy to clipboard', error instanceof Error ? error : new Error(errorMessage));
  };

  const handleClick = () => navigator.clipboard.writeText(value).then(onSuccess, onError);

  return (
    <Button
      {...props} // spreading props for tooltip to work
      icon={SVG}
      color="transparent"
      onClick={handleClick}
      size="x-small"
      className={cx(styles.button, className)}
    />
  );
}

export { CopyButton };
