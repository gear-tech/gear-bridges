import { useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';

import { SVGComponent } from '@/types';
import { logger } from '@/utils';

import CopySVG from './copy.svg?react';

type Props = {
  value: string;
  message?: string;
  SVG?: SVGComponent;
  className?: string;
  onCopy?: () => void;
};

function CopyButton({ value, message = 'Copied', SVG = CopySVG, className, onCopy = () => {} }: Props) {
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

  const copyToClipboard = () => navigator.clipboard.writeText(value).then(onSuccess, onError);

  return <Button icon={SVG} color="transparent" onClick={copyToClipboard} size="x-small" className={className} />;
}

export { CopyButton };
