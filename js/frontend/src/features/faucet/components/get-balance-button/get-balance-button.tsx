import { HexString } from '@gear-js/api';
import { useAccount, useAlert, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { Turnstile, TurnstileInstance } from '@marsidev/react-turnstile';
import { captureException } from '@sentry/react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';

import { LinkButton } from '@/components';
import { useNetworkType } from '@/context/network-type';
import { useEthAccount, useModal } from '@/hooks';
import { cx } from '@/utils';

import { GetBalanceParameters, getEthTokenBalance, getVaraAccountBalance } from '../../api';
import GiftSVG from '../../assets/gift.svg?react';
import { TURNSTILE_SITEKEY } from '../../consts';
import {
  ISOLATED_CHAIN_ID,
  ISOLATED_USDC,
  IsolatedAsset,
  getIsolatedConfig,
  getIsolatedStatus,
  requestIsolatedBalance,
} from '../../isolated-api';

import styles from './get-balance-button.module.scss';

type Props<T> = {
  getBalance: (parameters: GetBalanceParameters<T>) => Promise<unknown>;
  onSuccess?: () => void; // optional because vara account balance has a subscription and doesn't need to be refetched
} & T;

const BUTTON_PROPS = {
  icon: GiftSVG,
  text: 'Get Balance',
  size: 'x-small',
  className: styles.button,
} as const;

const ISOLATED_FAUCET = import.meta.env.VITE_ISOLATED_FAUCET === 'true';

function IsolatedBalanceButton({
  asset,
  address,
  genesis,
  chainId,
  onSuccess,
}: {
  asset: IsolatedAsset;
  address: string;
  genesis?: string;
  chainId?: number;
  onSuccess?: () => void;
}) {
  const { isTestnet, syncEthWalletNetwork } = useNetworkType();
  const queryClient = useQueryClient();
  const [requestError, setRequestError] = useState<string>();
  const [isOutcomeUnknown, setIsOutcomeUnknown] = useState(false);
  const [now, setNow] = useState(Date.now);
  const notifiedHash = useRef<string | undefined>(undefined);
  const config = useQuery({
    queryKey: ['isolated-faucet', 'config'],
    queryFn: getIsolatedConfig,
    enabled: isTestnet,
    staleTime: Infinity,
    retry: false,
  });
  const isNetworkReady = Boolean(
    isTestnet &&
      config.data &&
      genesis === config.data.genesis &&
      (asset === 'TVARA' || chainId === config.data.chainId),
  );
  const queryKey = ['isolated-faucet', 'status', genesis, chainId, asset, address];
  const status = useQuery({
    queryKey,
    queryFn: () => getIsolatedStatus(asset, address),
    enabled: isNetworkReady,
    retry: false,
    refetchInterval: (query) => (query.state.data?.status === 'pending' ? 3000 : false),
  });
  const request = useMutation({
    mutationFn: () => {
      if (!isNetworkReady || !config.data) throw new Error('The approved test network is not connected');

      return requestIsolatedBalance(asset, address, config.data);
    },
    retry: false,
  });
  const walletNetwork = useMutation({
    mutationFn: async () => {
      await syncEthWalletNetwork();
    },
    retry: false,
  });
  const claim = status.data;
  const nextClaimAt = claim?.nextClaimAt;
  const isCoolingDown = nextClaimAt !== undefined && nextClaimAt > now;

  useEffect(() => {
    if (claim?.status !== 'finalized' || !claim.txHash || notifiedHash.current === claim.txHash) return;

    notifiedHash.current = claim.txHash;
    onSuccess?.();
  }, [claim?.status, claim?.txHash, onSuccess]);

  useEffect(() => {
    setNow(Date.now());
    if (nextClaimAt === undefined || nextClaimAt <= Date.now()) return;

    const timeout = setTimeout(() => setNow(Date.now()), nextClaimAt - Date.now() + 100);

    return () => clearTimeout(timeout);
  }, [nextClaimAt]);

  const checkStatus = async () => {
    const result = await status.refetch();

    if (result.isSuccess) setIsOutcomeUnknown(false);
  };

  const handleClick = async () => {
    setRequestError(undefined);
    setIsOutcomeUnknown(true);

    try {
      const pending = await request.mutateAsync();

      await queryClient.cancelQueries({ queryKey, exact: true });

      queryClient.setQueryData(queryKey, pending);
      setIsOutcomeUnknown(false);
      setNow(Date.now());
    } catch (error) {
      setRequestError(error instanceof Error ? error.message : 'Request could not be confirmed');
      // A lost response can follow a broadcast. Read the original claim; never retry the POST automatically.
      await checkStatus();
    }
  };

  const networkError = !genesis
    ? 'Waiting for the Gear connection.'
    : config.data && genesis !== config.data.genesis
      ? 'Faucet unavailable: connected Gear genesis does not match the approved isolated chain.'
      : asset === 'USDC' && chainId !== ISOLATED_CHAIN_ID
        ? chainId === undefined
          ? 'Wallet connected, but its network is unavailable. Switch it to Hoodi (560048).'
          : `Wallet connected on chain ${chainId}. Switch it to Hoodi (560048) to request Test USDC.`
        : undefined;
  const label = asset === 'TVARA' ? '100 TVARA / 24h' : '1,000 Test USDC / 24h';

  return (
    <div className={styles.isolated}>
      <Button
        {...BUTTON_PROPS}
        text={`Get ${asset === 'TVARA' ? 'TVARA' : 'Test USDC'}`}
        onClick={() => void handleClick()}
        isLoading={request.isPending || claim?.status === 'pending'}
        disabled={
          !isNetworkReady ||
          !status.isSuccess ||
          status.isFetching ||
          request.isPending ||
          isOutcomeUnknown ||
          claim?.status === 'pending' ||
          isCoolingDown
        }
      />
      <span>{label} per address. Test-only tokens — no monetary value.</span>
      <div role="status" aria-live="polite">
        {config.isPending ? <p>Loading isolated faucet configuration…</p> : null}
        {config.error ? <p>Isolated faucet unavailable: {config.error.message}</p> : null}
        {networkError ? <p>{networkError}</p> : null}
        {walletNetwork.error ? <p>Network switch failed: {walletNetwork.error.message}</p> : null}
        {isNetworkReady && status.isPending ? <p>Checking existing claim…</p> : null}
        {status.error ? <p>Status unavailable: {status.error.message}. Do not assume a request failed.</p> : null}
        {requestError ? <p>Request error: {requestError}</p> : null}
        {isOutcomeUnknown ? <p>Request outcome unknown. Check status before requesting again.</p> : null}
        {claim?.status === 'idle' ? <p>No claim currently recorded.</p> : null}
        {claim?.status === 'pending' ? <p>Pending — waiting for finalized confirmation.</p> : null}
        {claim?.status === 'finalized' ? <p>Claim finalized. Tokens sent to your connected address.</p> : null}
        {claim?.status === 'failed' ? (
          <p>Claim failed: {claim.error ?? 'See the original transaction outcome.'}</p>
        ) : null}
        {claim?.txHash ? <p className={styles.transaction}>Transaction: {claim.txHash}</p> : null}
        {isCoolingDown && nextClaimAt !== undefined ? (
          <p>Next claim available: {new Date(nextClaimAt).toLocaleString()}</p>
        ) : null}
      </div>
      {asset === 'USDC' && isTestnet && chainId !== ISOLATED_CHAIN_ID ? (
        <Button
          text="Switch wallet to Hoodi"
          size="x-small"
          isLoading={walletNetwork.isPending}
          disabled={walletNetwork.isPending || request.isPending}
          onClick={() => walletNetwork.mutate()}
        />
      ) : null}
      {config.isError ? (
        <Button text="Reload faucet configuration" size="x-small" onClick={() => void config.refetch()} />
      ) : null}
      {isNetworkReady && (status.isError || requestError || isOutcomeUnknown) ? (
        <Button
          text="Check claim status"
          size="x-small"
          disabled={status.isFetching || request.isPending}
          onClick={() => void checkStatus()}
        />
      ) : null}
    </div>
  );
}

function ButtonComponent<T>({ getBalance, onSuccess, ...parameters }: Props<T>) {
  const alert = useAlert();
  const turnstileRef = useRef<TurnstileInstance>(null);

  const [isVerifying, setIsVerifying] = useState(false);
  const [isVerificationVisible, openVerification, closeVerification] = useModal();

  const { mutateAsync, isPending } = useMutation({
    mutationFn: (token: string) => getBalance({ token, ...(parameters as T) }),
  });

  const handleClick = () => {
    setIsVerifying(true);

    turnstileRef.current?.reset();
    turnstileRef.current?.execute();
  };

  const settleVerification = () => {
    closeVerification();
    setIsVerifying(false);
  };

  const handleVerificationSuccess = (token: string) => {
    settleVerification();

    mutateAsync(token)
      .then(() => {
        onSuccess?.();

        alert.success(
          'Your request for test tokens has been received and is being processed. The tokens will appear in your balance shortly.',
        );
      })
      .catch((error) => {
        alert.error(error instanceof Error ? error.message : String(error));
        captureException(error, { tags: { feature: 'faucet', flow: 'request' } });
      });
  };

  const handleVerificationError = (code: string) => {
    settleVerification();
    alert.error(`Error verifying that you are a human. Please try again.`);

    const error = new Error(`Cloudflare Turnstile (human verification) error. Code: ${code}`);

    captureException(error, { tags: { feature: 'faucet', flow: 'verification' } });
  };

  if (!TURNSTILE_SITEKEY)
    return (
      <span>
        <Button disabled {...BUTTON_PROPS} />
        <span>Faucet unavailable: human verification is not configured for this deployment.</span>
      </span>
    );

  return (
    <>
      <Button onClick={handleClick} isLoading={isPending || isVerifying} {...BUTTON_PROPS} />

      <div className={cx(styles.overlay, isVerificationVisible && styles.active)}>
        <Turnstile
          options={{ execution: 'execute', appearance: 'interaction-only' }}
          siteKey={TURNSTILE_SITEKEY}
          ref={turnstileRef}
          onBeforeInteractive={openVerification}
          onAfterInteractive={settleVerification}
          onError={handleVerificationError}
          onSuccess={handleVerificationSuccess}
        />
      </div>
    </>
  );
}

function GetVaraAccountBalanceButton() {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const { isMainnet, isTestnet } = useNetworkType();

  if (isMainnet || !api || !account) return;

  if (ISOLATED_FAUCET) {
    if (!isTestnet) return;

    return (
      <IsolatedBalanceButton
        key={`${api.genesisHash.toHex()}:${account.decodedAddress}`}
        asset="TVARA"
        address={account.address}
        genesis={isApiReady ? api.genesisHash.toHex() : undefined}
      />
    );
  }

  return (
    <ButtonComponent getBalance={getVaraAccountBalance} address={account.address} genesis={api.genesisHash.toHex()} />
  );
}

function GetEthTokenBalanceButton({
  address,
  symbol,
  onSuccess,
}: {
  address: HexString;
  symbol: string;
  onSuccess: () => void;
}) {
  const ethAccount = useEthAccount();
  const { api, isApiReady } = useApi();
  const { isMainnet, isTestnet } = useNetworkType();

  if (isMainnet || !ethAccount.address) return;

  const lowerCaseSymbol = symbol.toLowerCase();

  if (lowerCaseSymbol.includes('eth'))
    return (
      <LinkButton
        type="external"
        to="https://cloud.google.com/application/web3/faucet/ethereum/hoodi"
        {...BUTTON_PROPS}
      />
    );

  if (ISOLATED_FAUCET) {
    if (!isTestnet || address.toLowerCase() !== ISOLATED_USDC) return;

    return (
      <IsolatedBalanceButton
        key={`${api?.genesisHash.toHex()}:${ethAccount.address}:${ethAccount.chainId}`}
        asset="USDC"
        address={ethAccount.address}
        genesis={isApiReady ? api?.genesisHash.toHex() : undefined}
        chainId={ethAccount.chainId}
        onSuccess={onSuccess}
      />
    );
  }

  if (!lowerCaseSymbol.includes('usdc') && !lowerCaseSymbol.includes('usdt') && !lowerCaseSymbol.includes('btc'))
    return;

  return (
    <ButtonComponent
      getBalance={getEthTokenBalance}
      address={ethAccount.address}
      contract={address}
      onSuccess={onSuccess}
    />
  );
}

const GetBalanceButton = {
  VaraAccount: GetVaraAccountBalanceButton,
  EthToken: GetEthTokenBalanceButton,
};

export { GetBalanceButton };
