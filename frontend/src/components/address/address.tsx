import { ComponentProps } from 'react';

import { TruncatedText } from '../layout';
import { Tooltip } from '../tooltip';

type Props = {
  value: string;
  className?: string;
  tooltip?: { side?: ComponentProps<typeof Tooltip>['side'] };
};

function Address({ value, className, tooltip }: Props) {
  return (
    <Tooltip value={value} {...tooltip}>
      <TruncatedText value={value} className={className} />
    </Tooltip>
  );
}

export { Address };
