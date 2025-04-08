import { TruncatedText } from '../layout';
import { Tooltip } from '../tooltip';

type Props = {
  value: string;
  className?: string;
};

function Address({ value, className }: Props) {
  return (
    <Tooltip value={value}>
      <TruncatedText value={value} className={className} />
    </Tooltip>
  );
}

export { Address };
