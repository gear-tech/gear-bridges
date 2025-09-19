import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';

import { Values, FormattedValues } from './form';
import { UseAccountBalance, UseFTBalance, UseFee, UseSendTxs, UseTxsEstimate } from './hooks';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

export type { UseAccountBalance, UseFTBalance, UseFee, UseSendTxs, UseTxsEstimate, Values, FormattedValues, Extrinsic };
