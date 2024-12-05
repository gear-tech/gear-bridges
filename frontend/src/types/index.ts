import { HexString } from '@gear-js/api';
import { FunctionComponent, SVGProps } from 'react';

type SVGComponent = FunctionComponent<
  SVGProps<SVGSVGElement> & {
    title?: string | undefined;
  }
>;

type VaraAddress = HexString;
type EthAddress = HexString;
type FTAddressPair = [VaraAddress, EthAddress];

export type { SVGComponent, FTAddressPair };
