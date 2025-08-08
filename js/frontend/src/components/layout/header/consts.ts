import { ROUTE } from '@/consts';

const LINKS = {
  [ROUTE.HOME]: 'Bridge',
  [ROUTE.TRANSACTIONS]: 'Transactions',
  [ROUTE.TOKEN_TRACKER]: 'My Tokens',
  [ROUTE.FAQ]: 'FAQ',
} as const;

export { LINKS };
