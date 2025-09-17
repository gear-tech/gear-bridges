import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';

import { Values, FormattedValues, UseHandleSubmitParameters, UseHandleSubmit } from './form';
import { UseAccountBalance, UseFTBalance, UseFee } from './hooks';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

export type {
  UseAccountBalance,
  UseFTBalance,
  UseHandleSubmitParameters,
  UseHandleSubmit,
  UseFee,
  Values,
  FormattedValues,
  Extrinsic,
};
