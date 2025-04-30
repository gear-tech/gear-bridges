import { HexString } from '@gear-js/api';

import { fetchWithGuard } from '@/utils';

const API_URL = import.meta.env.VITE_FAUCET_API_URL as string;

type VaraAccountBalance = {
  address: string;
  genesis: HexString;
};

type EthTokenBalance = {
  address: HexString;
  contract: HexString;
};

type GetBalanceParameters<T> = {
  token: string; // hCaptcha token
  payload: T;
};

const getVaraAccountBalance = (parameters: GetBalanceParameters<VaraAccountBalance>) =>
  fetchWithGuard({ url: `${API_URL}/balance`, method: 'POST', parameters });

const getEthTokenBalance = (parameters: GetBalanceParameters<EthTokenBalance>) =>
  fetchWithGuard({
    url: `${API_URL}/bridge/request`,
    method: 'POST',
    parameters,
  });

export { getVaraAccountBalance, getEthTokenBalance };
export type { VaraAccountBalance, EthTokenBalance, GetBalanceParameters };
