import { getTypedEntries } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useMemo } from 'react';
import { FormProvider, useForm } from 'react-hook-form';
import { isHex } from 'viem';
import { z } from 'zod';

import { Container, Input, Select, Skeleton } from '@/components';
import { NETWORK_NAME, SPEC } from '@/consts';
import { NETWORK_NAME_DIRECTION, useTransactions, useTransactionsCount } from '@/features/history';
import { TransactionCard } from '@/features/history/components/transaction-card';
import { Direction, Pair, Status, TeleportWhereInput } from '@/features/history/graphql/graphql';
import { asOptionalField } from '@/utils';

import { List } from './list';
import SearchSVG from './search.svg?react';
import styles from './transactions.module.scss';
import { useDebounce } from './use-debounce';

const TRANSACTIONS_LIMIT = 12;

const getLastDaysISOTimestamp = (daysCount: number) =>
  new Date(Date.now() - daysCount * 24 * 60 * 60 * 1000).toISOString();

const TIMESTAMP_OPTIONS = [
  { label: 'All Time', value: '' },
  { label: 'Last 7 days', value: getLastDaysISOTimestamp(7) },
  { label: 'Last 30 days', value: getLastDaysISOTimestamp(30) },
  { label: 'Last 90 days', value: getLastDaysISOTimestamp(90) },
  { label: 'Last 180 days', value: getLastDaysISOTimestamp(180) },
  { label: 'Last 365 days', value: getLastDaysISOTimestamp(365) },
] as const;

const STATUS_OPTIONS = [
  { label: 'All Statuses', value: '' },
  { label: 'Completed', value: Status.Completed },
  { label: 'In Progress', value: Status.InProgress },
];

const SYMBOL_OPTIONS = getTypedEntries(SPEC).flatMap(([pair, bridge]) => {
  const varaBridge = bridge[NETWORK_NAME.VARA];
  const ethBridge = bridge[NETWORK_NAME.ETH];

  const varaDirection = NETWORK_NAME_DIRECTION[NETWORK_NAME.VARA];
  const ethDirection = NETWORK_NAME_DIRECTION[NETWORK_NAME.ETH];

  return [
    {
      label: `${varaBridge.symbol} → ${ethBridge.symbol}`,
      value: `${pair}.${varaDirection}` as const,
    },
    {
      label: `${ethBridge.symbol} → ${varaBridge.symbol}`,
      value: `${pair}.${ethDirection}` as const,
    },
  ];
});

const ASSET_OPTIONS = [{ label: 'All Assets', value: '' as const }, ...SYMBOL_OPTIONS];

type AssetValue = (typeof ASSET_OPTIONS)[number]['value'];

const FIELD_NAME = {
  TIMESTAMP: 'timestamp',
  STATUS: 'status',
  ASSET: 'asset',
  SEARCH: 'search',
} as const;

const DEFAULT_VALUES = {
  [FIELD_NAME.TIMESTAMP]: TIMESTAMP_OPTIONS[0].value,
  [FIELD_NAME.STATUS]: STATUS_OPTIONS[0].value as Status | '',
  [FIELD_NAME.ASSET]: ASSET_OPTIONS[0].value,
  [FIELD_NAME.SEARCH]: '',
};

const getTransactionFilters = (timestamp: string, status: Status | '', asset: AssetValue, blockhash: string) => {
  const where = {} as TeleportWhereInput;

  if (timestamp) where.timestamp_gt = timestamp;
  if (status) where.status_eq = status;

  if (asset) {
    const [pair, direction] = asset.split('.') as [Pair, Direction];

    where.pair_eq = pair;
    where.direction_eq = direction;
  }

  if (blockhash) where.blockhash_eq = blockhash;

  return where;
};

const SCHEMA = z.object({
  [FIELD_NAME.SEARCH]: asOptionalField(
    z
      .string()
      .trim()
      .refine((value) => isHex(value), { message: 'Value should be hex' }),
  ),
});

function Transactions() {
  const form = useForm({ defaultValues: DEFAULT_VALUES, mode: 'onChange', resolver: zodResolver(SCHEMA) });
  const { watch, formState } = form;

  const timestamp = watch(FIELD_NAME.TIMESTAMP);
  const status = watch(FIELD_NAME.STATUS);
  const asset = watch(FIELD_NAME.ASSET);

  // treat carefully, formState is not acting 100% accurate with watch and optional valition schema.
  // relying on it only cuz debounce value is getting set later. otherwise would need to think about another solution
  const searchError = formState.errors[FIELD_NAME.SEARCH];
  const search = watch(FIELD_NAME.SEARCH);
  const [debouncedSearch] = useDebounce(search, 300);

  const filters = useMemo(
    () => getTransactionFilters(timestamp, status, asset, searchError ? '' : debouncedSearch),
    [timestamp, status, asset, searchError, debouncedSearch],
  );

  const [transactionsCount, isTransactionsCountLoading] = useTransactionsCount(filters);
  const [transactions, isFetching, hasNextPage, fetchNextPage] = useTransactions(transactionsCount, filters);

  return (
    <Container>
      <header>
        <FormProvider {...form}>
          <form className={styles.filters} onSubmit={(e) => e.preventDefault()}>
            <Select name={FIELD_NAME.TIMESTAMP} label="Date" options={TIMESTAMP_OPTIONS} />
            <Select name={FIELD_NAME.STATUS} label="Status" options={STATUS_OPTIONS} />
            <Select name={FIELD_NAME.ASSET} label="Asset" options={ASSET_OPTIONS} />
            <Input name={FIELD_NAME.SEARCH} label="Search" icon={SearchSVG} />
          </form>
        </FormProvider>

        <p className={styles.counter}>
          {isTransactionsCountLoading ? <Skeleton width="100px" /> : `${transactionsCount} results`}
        </p>
      </header>

      <List
        items={transactions}
        hasMore={hasNextPage}
        renderItem={(transaction) => <TransactionCard {...transaction} />}
        fetchMore={fetchNextPage}
        skeleton={{
          rowsCount: TRANSACTIONS_LIMIT,
          isVisible: isTransactionsCountLoading || isFetching,
          renderItem: () => <TransactionCard.Skeleton />,
        }}
      />
    </Container>
  );
}

export { Transactions };
