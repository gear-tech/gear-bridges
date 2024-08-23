import { FunctionComponent, SVGProps } from 'react';

import { Pair } from './spec';

type SVGComponent = FunctionComponent<
  SVGProps<SVGSVGElement> & {
    title?: string | undefined;
  }
>;

export type { SVGComponent, Pair };
