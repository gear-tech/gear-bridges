import { HexString } from '@gear-js/api';
import { useAccount } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useMemo } from 'react';
import { useForm } from 'react-hook-form';
import { useSearchParams } from 'react-router-dom';
import { z } from 'zod';

import { useDebounce, useEthAccount } from '@/hooks';
import { asOptionalField, isNumeric } from '@/utils';

import { DEFAULT_VALUES, FIELD_NAME } from '../consts';
import { Status, TransferFilter } from '../types';

const SCHEMA = z.object({
  [FIELD_NAME.TIMESTAMP]: z.string(),
  [FIELD_NAME.STATUS]: asOptionalField(
    z.enum([Status.Completed, Status.Failed, Status.AwaitingPayment, Status.Bridging]),
  ),
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

  const [searchParams] = useSearchParams();

  const getOwnerDefaultValue = () => {
    if (!accountAddress) return DEFAULT_VALUES[FIELD_NAME.OWNER];

    const owner = searchParams.get('owner');

    if (owner === 'true') return true;
    if (owner === 'false') return false;

    return DEFAULT_VALUES[FIELD_NAME.OWNER];
  };

  const getStatusDefaultValue = () => {
    const status = searchParams.get('status') as Status;

    if (Object.values(Status).includes(status)) return status;

    return DEFAULT_VALUES[FIELD_NAME.STATUS];
  };

  const form = useForm({
    defaultValues: {
      // TODO: search params for each field, field should change param on change
      ...DEFAULT_VALUES,
      [FIELD_NAME.OWNER]: getOwnerDefaultValue(),
      [FIELD_NAME.STATUS]: getStatusDefaultValue(),
    },
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
    const filter = {} as TransferFilter;

    if (timestamp) {
      filter.timestamp = { greaterThan: timestamp } as TransferFilter['timestamp'];
    }

    if (status) {
      filter.status = { equalTo: status } as TransferFilter['status'];
    }

    if (asset) {
      const [source, destination] = asset.split('.') as [HexString, HexString];

      filter.source = { equalTo: source } as TransferFilter['source'];
      filter.destination = { equalTo: destination } as TransferFilter['destination'];
    }

    if (debouncedSearch && !searchError) {
      filter.blockNumber = { equalTo: debouncedSearch } as TransferFilter['blockNumber'];
    }

    if (owner && accountAddress) {
      filter.sender = { includesInsensitive: accountAddress } as TransferFilter['sender'];
    }

    if (Object.keys(filter).length === 0) return;

    return filter;
  }, [timestamp, status, asset, searchError, debouncedSearch, owner, accountAddress]);

  return { form, filters };
}

export { useTransactionFilters };
