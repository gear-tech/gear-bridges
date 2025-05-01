import { useQuery } from '@tanstack/react-query';

import { fetchWithGuard } from '@/utils';

const API_URL = import.meta.env.VITE_TOKEN_PRICE_API_URL as string;

const TOKEN_ID = {
  VARA: 'vara-network',
  ETH: 'ethereum',
  USDT: 'tether',
  USDC: 'usd-coin',
} as const;

const PARAMS = new URLSearchParams({
  ids: Object.values(TOKEN_ID).join(','),
  vs_currencies: 'usd',
});

type TokenId = (typeof TOKEN_ID)[keyof typeof TOKEN_ID];

type Test = {
  [Key in TokenId]: { usd: number };
};

const getTokenPrices = () => fetchWithGuard<Test>({ url: `${API_URL}?${PARAMS.toString()}` });

function useTokenPrices() {
  return useQuery({
    queryKey: ['tokenPrice'],
    queryFn: getTokenPrices,
  });
}

export { TOKEN_ID, useTokenPrices };
export type { TokenId };
