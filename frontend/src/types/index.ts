import { FunctionComponent, SVGProps } from 'react';

type SVGComponent = FunctionComponent<
  SVGProps<SVGSVGElement> & {
    title?: string | undefined;
  }
>;

export type { SVGComponent };
