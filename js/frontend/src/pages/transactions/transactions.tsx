import { useAccount } from '@gear-js/react-hooks';
import { useMemo } from 'react';
import { FormProvider } from 'react-hook-form';

import SearchSVG from '@/assets/search.svg?react';
import { Container, Input, Select, Skeleton, Checkbox } from '@/components';
import { useTokens } from '@/context';
import {
  useTransactions,
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
import { useEthAccount, useVaraSymbol } from '@/hooks';

import styles from './transactions.module.scss';

function Transactions() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isAccountConnected = Boolean(account || ethAccount.address);

  const { form, filters } = useTransactionFilters();
  const [txsData, isFetching, hasNextPage, fetchNextPage] = useTransactions(filters);
  const { transactions, transactionsCount } = txsData || {};

  const varaSymbol = useVaraSymbol();
  const { data: pairs, isLoading } = usePairs();
  const assetOptions = useMemo(() => getAssetOptions(pairs || [], varaSymbol || 'TVARA'), [pairs, varaSymbol]);

  const { pairHashToToken } = useTokens();

  return (
    <Container>
      <header>
        <FormProvider {...form}>
          <form onSubmit={(e) => e.preventDefault()}>
            <div className={styles.filters}>
              <Select name={FIELD_NAME.TIMESTAMP} label="Date" options={TIMESTAMP_OPTIONS} />
              <Select name={FIELD_NAME.STATUS} label="Status" options={STATUS_OPTIONS} />

              <Select
                name={FIELD_NAME.ASSET}
                label="Asset"
                options={assetOptions}
                disabled={isLoading || !varaSymbol}
              />

              <Input name={FIELD_NAME.SEARCH} label="Search (Block Number)" icon={SearchSVG} />
            </div>

            <p className={styles.counter}>
              {isFetching ? <Skeleton width="100px" /> : `${transactionsCount} results`}

              {isAccountConnected && (
                <Checkbox name={FIELD_NAME.OWNER} type="switch" label="My Transactions" className={styles.switch} />
              )}
            </p>
          </form>
        </FormProvider>
      </header>

      <List
        items={pairHashToToken ? transactions : undefined}
        hasMore={hasNextPage}
        renderItem={(transaction) => <TransactionCard {...transaction} pairHashToToken={pairHashToToken!} />}
        fetchMore={fetchNextPage}
        skeleton={{
          rowsCount: TRANSACTIONS_LIMIT,
          isVisible: isFetching || isLoading,
          renderItem: () => <TransactionCard.Skeleton />,
        }}
      />
    </Container>
  );
}

export { Transactions };
