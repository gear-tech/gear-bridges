import { HexString } from '@gear-js/api';

import { fetchWithGuard } from '@/utils';

const API_URL = import.meta.env.VITE_FAUCET_API_URL as string;

const getErrorMessage = async (response: Response) => {
  const result = (await response.json()) as unknown;

  if (result !== null && typeof result === 'object' && 'error' in result && typeof result.error === 'string')
    return result.error;
};

const FETCH_PARAMETERS = {
  method: 'POST',
  getErrorMessage,
  isJson: false,
} as const;

type VaraPayload = {
  address: string;
  genesis: HexString;
};

type EthPayload = {
  address: HexString;
  contract: HexString;
};

type GetBalanceParameters<T> = {
  token: string; // hCaptcha token
  payload: T;
};

const getVaraAccountBalance = (parameters: GetBalanceParameters<VaraPayload>) =>
  fetchWithGuard({ ...FETCH_PARAMETERS, url: `${API_URL}/balance`, parameters });

const getEthTokenBalance = (parameters: GetBalanceParameters<EthPayload>) =>
  fetchWithGuard({ ...FETCH_PARAMETERS, url: `${API_URL}/bridge/request`, parameters });

export { getVaraAccountBalance, getEthTokenBalance };
export type { GetBalanceParameters };
