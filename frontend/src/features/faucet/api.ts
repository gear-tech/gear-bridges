import { HexString } from '@gear-js/api';

import { fetchWithGuard } from '@/utils';

const API_URL = import.meta.env.VITE_FAUCET_API_URL as string;

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

const getErrorMessage = async (response: Response) => {
  const result = (await response.json()) as unknown;

  if (result !== null && typeof result === 'object' && 'error' in result && typeof result.error === 'string')
    return result.error;
};

const getVaraAccountBalance = (parameters: GetBalanceParameters<VaraPayload>) =>
  fetchWithGuard({ url: `${API_URL}/balance`, method: 'POST', parameters, getErrorMessage });

const getEthTokenBalance = (parameters: GetBalanceParameters<EthPayload>) =>
  fetchWithGuard({
    url: `${API_URL}/bridge/request`,
    method: 'POST',
    parameters,
    getErrorMessage,
  });

export { getVaraAccountBalance, getEthTokenBalance };
export type { GetBalanceParameters };
