import { HexString } from '@gear-js/api';
import { useAccount } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useMemo } from 'react';
import { useForm } from 'react-hook-form';
import { z } from 'zod';

import { useDebounce, useEthAccount } from '@/hooks';
import { asOptionalField, isNumeric } from '@/utils';

import { DEFAULT_VALUES, FIELD_NAME } from '../consts';
import { Status, TransferWhereInput } from '../types';

const SCHEMA = z.object({
  [FIELD_NAME.TIMESTAMP]: z.string(),
  [FIELD_NAME.STATUS]: asOptionalField(z.enum([Status.Completed, Status.Failed, Status.Pending, Status.InProgress])),
  [FIELD_NAME.ASSET]: z.string(),

  [FIELD_NAME.SEARCH]: asOptionalField(
    z
      .string()
      .trim()
      .refine((value) => isNumeric(value), { message: 'Value should be a number' }),
  ),

  [FIELD_NAME.OWNER]: z.boolean(),
});

function useTransactionFilters() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const accountAddress = account?.decodedAddress || ethAccount.address;

  const form = useForm({
    defaultValues: DEFAULT_VALUES,
    mode: 'onChange',
    resolver: zodResolver(SCHEMA),
  });

  const { watch, formState } = form;

  const timestamp = watch(FIELD_NAME.TIMESTAMP);
  const status = watch(FIELD_NAME.STATUS);
  const asset = watch(FIELD_NAME.ASSET);
  const search = watch(FIELD_NAME.SEARCH);
  const owner = watch(FIELD_NAME.OWNER);

  // treat carefully, formState is not acting 100% accurate with watch and optional valition schema.
  // relying on it only cuz debounce value is getting set later. otherwise would need to think about another solution
  const searchError = formState.errors[FIELD_NAME.SEARCH];
  const [debouncedSearch] = useDebounce(search, 300);

  const filters = useMemo(() => {
    const where = {} as TransferWhereInput;

    if (timestamp) where.timestamp_gt = timestamp;
    if (status) where.status_eq = status;

    if (asset) {
      const [source, destination] = asset.split('.') as [HexString, HexString];

      where.source_eq = source;
      where.destination_eq = destination;
    }

    if (debouncedSearch && !searchError) where.blockNumber_eq = debouncedSearch;
    if (owner && accountAddress) where.sender_eq = accountAddress;

    return where;
  }, [timestamp, status, asset, searchError, debouncedSearch, owner, accountAddress]);

  return { form, filters };
}

export { useTransactionFilters };
