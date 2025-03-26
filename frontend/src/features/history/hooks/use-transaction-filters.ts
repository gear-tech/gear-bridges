import { HexString } from '@gear-js/api';
import { zodResolver } from '@hookform/resolvers/zod';
import { useMemo } from 'react';
import { useForm } from 'react-hook-form';
import { z } from 'zod';

import { useDebounce } from '@/hooks';
import { asOptionalField, isNumeric } from '@/utils';

import { DEFAULT_VALUES, FIELD_NAME } from '../consts';
import { TransferWhereInput } from '../types';

const SCHEMA = z.object({
  [FIELD_NAME.SEARCH]: asOptionalField(
    z
      .string()
      .trim()
      .refine((value) => isNumeric(value), { message: 'Value should be a number' }),
  ),
});

function useTransactionFilters() {
  const form = useForm<typeof DEFAULT_VALUES, unknown, z.infer<typeof SCHEMA>>({
    defaultValues: DEFAULT_VALUES,
    mode: 'onChange',

    // @ts-expect-error -- revisit after next pr are released:
    // https://github.com/react-hook-form/react-hook-form/pull/12638
    // https://github.com/react-hook-form/resolvers/pull/753
    resolver: zodResolver(SCHEMA),
  });

  const { watch, formState } = form;

  const timestamp = watch(FIELD_NAME.TIMESTAMP);
  const status = watch(FIELD_NAME.STATUS);
  const asset = watch(FIELD_NAME.ASSET);
  const search = watch(FIELD_NAME.SEARCH);

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

    return where;
  }, [timestamp, status, asset, searchError, debouncedSearch]);

  return { form, filters };
}

export { useTransactionFilters };
