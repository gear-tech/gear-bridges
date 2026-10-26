import { Turnstile } from '@marsidev/react-turnstile';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { z } from 'zod';

import { getEthTokenBalance, getVaraAccountBalance } from './api';
import { TURNSTILE_SITEKEY } from './consts';
import { getIsolatedConfig, getIsolatedStatus, requestIsolatedBalance } from './isolated-api';
import type { IsolatedAsset } from './isolated-api';

const ISOLATED = import.meta.env.VITE_ISOLATED_FAUCET === 'true';
const hex = (bytes: number) =>
  z
    .string()
    .regex(new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`))
    .refine((value) => !/^0x0+$/.test(value), 'Zero identity is forbidden');
const publicProfile = z.object({
  genesis: hex(32),
  tokens: z.array(z.object({ name: z.string().min(1).max(80), address: hex(20) })),
});

function IsolatedFaucet({ asset, address }: { asset: IsolatedAsset; address: string }) {
  const queryClient = useQueryClient();
  const [uncertain, setUncertain] = useState(false);
  const [now, setNow] = useState(Date.now);
  const inFlight = useRef(false);
  const config = useQuery({ queryKey: ['faucet-config'], queryFn: getIsolatedConfig, retry: false });
  const queryKey = ['faucet-claim', asset, address];
  const status = useQuery({
    queryKey,
    queryFn: () => getIsolatedStatus(asset, address),
    enabled: Boolean(address && config.data),
    retry: false,
    refetchInterval: (query) => (query.state.data?.status === 'pending' ? 3000 : false),
  });
  const request = useMutation({
    mutationFn: () => {
      if (!config.data) throw new Error('Approved faucet configuration unavailable');
      return requestIsolatedBalance(asset, address, config.data);
    },
    retry: false,
  });
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const claim = status.data;
  const check = async () => {
    const result = await status.refetch();
    if (result.isSuccess) setUncertain(false);
  };
  const submit = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setUncertain(true);
    try {
      const pending = await request.mutateAsync();
      await queryClient.cancelQueries({ queryKey, exact: true });
      queryClient.setQueryData(queryKey, pending);
      setUncertain(false);
    } catch {
      // A lost response may follow broadcast: observe, never automatically retry the POST.
      await check();
    } finally {
      inFlight.current = false;
    }
  };
  const cooling = (claim?.nextClaimAt ?? 0) > now;
  return (
    <section>
      <p>
        {asset === 'TVARA' ? '100 TVARA' : '1,000 Test USDC'} per address per 24 hours. Hoodi / approved isolated Gear
        only.
      </p>
      <button
        type="button"
        onClick={() => void submit()}
        disabled={
          !address ||
          !status.isSuccess ||
          status.isFetching ||
          !config.data ||
          request.isPending ||
          uncertain ||
          claim?.status === 'pending' ||
          cooling
        }>
        Request test tokens
      </button>
      <button type="button" disabled={!address || status.isFetching || request.isPending} onClick={() => void check()}>
        Check original claim
      </button>
      <div role="status" aria-live="polite">
        {config.error && <p>Configuration unavailable: {config.error.message}</p>}
        {status.error && <p>Status unavailable: {status.error.message}. Do not assume a request failed.</p>}
        {request.error && <p>{request.error.message}</p>}
        {uncertain && <p>Outcome unknown. Check the original claim before requesting again.</p>}
        {claim && (
          <p>
            Claim: {claim.status}
            {claim.status === 'pending' ? ' — awaiting finalized confirmation' : ''}
          </p>
        )}
        {claim?.error && <p>{claim.error}</p>}
        {claim?.txHash && <p>Original transaction: {claim.txHash}</p>}
        {cooling && <p>Next claim: {new Date(claim!.nextClaimAt!).toLocaleString()}</p>}
      </div>
    </section>
  );
}

function PublicFaucet({ address, asset }: { address: string; asset: string }) {
  const [token, setToken] = useState('');
  const [outcomeUnknown, setOutcomeUnknown] = useState(false);
  const inFlight = useRef(false);
  const profile = publicProfile.parse(JSON.parse(import.meta.env.VITE_FAUCET_PROFILE || 'null'));
  const request = useMutation({
    mutationFn: () => {
      if (!token || !address) throw new Error('Recipient and human verification required');
      if (asset === 'VARA') return getVaraAccountBalance({ address, genesis: profile.genesis as `0x${string}`, token });
      const contract = profile.tokens.find((item) => item.address === asset)?.address;
      if (!contract || !/^0x[0-9a-fA-F]{40}$/.test(address) || /^0x0+$/.test(address))
        throw new Error('Select an approved test token and Ethereum recipient');
      return getEthTokenBalance({ address: address as `0x${string}`, contract: contract as `0x${string}`, token });
    },
    retry: false,
  });
  const submit = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setOutcomeUnknown(true);
    try {
      await request.mutateAsync();
    } catch {
      /* No automatic retry of a possibly accepted payout. */
    } finally {
      setToken('');
      inFlight.current = false;
    }
  };
  return (
    <section>
      <Turnstile
        siteKey={TURNSTILE_SITEKEY}
        onSuccess={setToken}
        onExpire={() => setToken('')}
        onError={() => setToken('')}
      />
      <button
        type="button"
        disabled={!address || !token || request.isPending || outcomeUnknown}
        onClick={() => void submit()}>
        Request test tokens
      </button>
      <div role="status" aria-live="polite">
        {request.isSuccess && (
          <p>Request accepted for processing, not proof of finalized payout. Check the recipient balance.</p>
        )}
        {request.error && (
          <p>{request.error.message}. Outcome may be unknown; check the recipient balance before a new request.</p>
        )}
      </div>
    </section>
  );
}

function Faucet() {
  const [address, setAddress] = useState('');
  const [asset, setAsset] = useState(ISOLATED ? 'TVARA' : 'VARA');
  const profile = ISOLATED ? undefined : publicProfile.parse(JSON.parse(import.meta.env.VITE_FAUCET_PROFILE || 'null'));
  return (
    <main>
      <h1>Test token faucet</h1>
      <p>
        Test-only tokens have no monetary value. This separate origin never asks you to sign or approve a bridge
        transaction.
      </p>
      <label>
        Test asset{' '}
        <select value={asset} onChange={(event) => setAsset(event.target.value)}>
          {ISOLATED ? (
            <>
              <option value="TVARA">TVARA (isolated Gear)</option>
              <option value="USDC">Test USDC (Hoodi)</option>
            </>
          ) : (
            <>
              <option value="VARA">Test VARA</option>
              {profile!.tokens.map((item) => (
                <option key={item.address} value={item.address}>
                  {item.name} ({item.address})
                </option>
              ))}
            </>
          )}
        </select>
      </label>
      <label>
        Recipient address{' '}
        <input
          autoComplete="off"
          maxLength={128}
          value={address}
          onChange={(event) => setAddress(event.target.value.trim())}
        />
      </label>
      {ISOLATED ? (
        <IsolatedFaucet key={`${asset}:${address}`} asset={asset as IsolatedAsset} address={address} />
      ) : (
        <PublicFaucet key={`${asset}:${address}`} asset={asset} address={address} />
      )}
      <p>
        <a
          href="https://cloud.google.com/application/web3/faucet/ethereum/hoodi"
          target="_blank"
          rel="noopener noreferrer">
          Get Hoodi ETH from the external Ethereum faucet
        </a>
      </p>
    </main>
  );
}

export { Faucet };
