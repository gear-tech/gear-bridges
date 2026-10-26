import type { HexString } from '@gear-js/api';

const getErrorMessage = async (response: Response) => {
  const result = (await response.json()) as unknown;

  if (result !== null && typeof result === 'object' && 'error' in result && typeof result.error === 'string')
    return result.error;
};

async function request(parameters: (VaraPayload & { token: string }) | (EthPayload & { token: string })) {
  const response = await fetch('/faucet/bridge/request', {
    method: 'POST',
    credentials: 'same-origin',
    mode: 'same-origin',
    redirect: 'error',
    cache: 'no-store',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(parameters),
  });
  if (!response.ok) throw new Error((await getErrorMessage(response)) || 'Faucet request could not be confirmed');
  return response;
}

type VaraPayload = {
  address: string;
  genesis: HexString;
};

type EthPayload = {
  address: HexString;
  contract: HexString;
};

type GetBalanceParameters<T> = {
  token: string; // Turnstile proof, validated by the existing payout service.
} & T;

const getVaraAccountBalance = (parameters: GetBalanceParameters<VaraPayload>) => request(parameters);

const getEthTokenBalance = (parameters: GetBalanceParameters<EthPayload>) => request(parameters);

export { getVaraAccountBalance, getEthTokenBalance };
export type { GetBalanceParameters };
