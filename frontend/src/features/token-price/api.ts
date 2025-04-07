import { useQuery } from '@tanstack/react-query';

const API_URL = import.meta.env.VITE_TOKEN_PRICE_API_URL as string;

const TOKEN_ID = {
  VARA: 'vara-network',
  ETH: 'ethereum',
  USDT: 'tether',
  USDC: 'usd-coin',
} as const;

const TOKEN_IDS = Object.values(TOKEN_ID);

const PRECISION = 3;

type TokenId = (typeof TOKEN_ID)[keyof typeof TOKEN_ID];

type Response = {
  [Key in TokenId]: { usd: number };
};

const getTokenPrices = async () => {
  const params = new URLSearchParams({
    ids: TOKEN_IDS.join(','),
    vs_currencies: 'usd',
    precision: PRECISION.toString(),
  });

  const url = `${API_URL}?${params.toString()}`;

  const response = await fetch(url);

  if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`);

  return (await response.json()) as Response;
};

function useTokenPrices() {
  return useQuery({
    queryKey: ['tokenPrice'],
    queryFn: getTokenPrices,
  });
}

export { TOKEN_ID, PRECISION, useTokenPrices };
