import { useAccount } from '@gear-js/react-hooks';
import { useMemo } from 'react';
import { FormProvider } from 'react-hook-form';

import SearchSVG from '@/assets/search.svg?react';
import { Container, Input, Select, Skeleton, Checkbox } from '@/components';
import { useTokens } from '@/context';
import {
  useTransactions,
  useTransactionsCount,
  List,
  FIELD_NAME,
  getAssetOptions,
  STATUS_OPTIONS,
  TIMESTAMP_OPTIONS,
  useTransactionFilters,
  TransactionCard,
  TRANSACTIONS_LIMIT,
  usePairs,
} from '@/features/history';
import { useEthAccount } from '@/hooks';

import styles from './transactions.module.scss';

function Transactions() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isAccountConnected = Boolean(account || ethAccount.address);

  const { form, filters } = useTransactionFilters();
  const [transactionsCount, isTransactionsCountLoading] = useTransactionsCount(filters);
  const [transactions, isFetching, hasNextPage, fetchNextPage] = useTransactions(transactionsCount, filters);

  const { data: pairs, isLoading } = usePairs();
  const assetOptions = useMemo(() => getAssetOptions(pairs || []), [pairs]);

  const { addressToToken } = useTokens();

  return (
    <Container>
      <header>
        <FormProvider {...form}>
          <form onSubmit={(e) => e.preventDefault()}>
            <div className={styles.filters}>
              <Select name={FIELD_NAME.TIMESTAMP} label="Date" options={TIMESTAMP_OPTIONS} />
              <Select name={FIELD_NAME.STATUS} label="Status" options={STATUS_OPTIONS} />
              <Select name={FIELD_NAME.ASSET} label="Asset" options={assetOptions} disabled={isLoading} />
              <Input name={FIELD_NAME.SEARCH} label="Search (Block Number)" icon={SearchSVG} />
            </div>

            <p className={styles.counter}>
              {isTransactionsCountLoading ? <Skeleton width="100px" /> : `${transactionsCount} results`}

              {isAccountConnected && (
                <Checkbox name={FIELD_NAME.OWNER} type="switch" label="My Transactions" className={styles.switch} />
              )}
            </p>
          </form>
        </FormProvider>
      </header>

      <List
        items={addressToToken ? transactions : undefined}
        hasMore={hasNextPage}
        renderItem={(transaction) => <TransactionCard {...transaction} addressToToken={addressToToken!} />}
        fetchMore={fetchNextPage}
        skeleton={{
          rowsCount: TRANSACTIONS_LIMIT,
          isVisible: isTransactionsCountLoading || isFetching || isLoading,
          renderItem: () => <TransactionCard.Skeleton />,
        }}
      />
    </Container>
  );
}

export { Transactions };
