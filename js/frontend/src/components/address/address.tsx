import { ComponentProps } from 'react';

import { getTruncatedText } from '@/utils';

import { Tooltip } from '../tooltip';

type Props = {
  value: string;
  prefixLength?: number;
  className?: string;
  tooltip?: { side?: ComponentProps<typeof Tooltip>['side'] };
};

function Address({ value, prefixLength, className, tooltip }: Props) {
  return (
    <Tooltip value={value} {...tooltip}>
      <span className={className}>{getTruncatedText(value, prefixLength)}</span>
    </Tooltip>
  );
}

export { Address };
