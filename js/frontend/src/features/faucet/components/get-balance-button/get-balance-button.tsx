import { HexString } from '@gear-js/api';
import { useAccount, useAlert, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { Turnstile, TurnstileInstance } from '@marsidev/react-turnstile';
import { captureException } from '@sentry/react';
import { useMutation } from '@tanstack/react-query';
import { useRef, useState } from 'react';

import { LinkButton } from '@/components';
import { useNetworkType } from '@/context/network-type';
import { useEthAccount, useModal } from '@/hooks';
import { cx } from '@/utils';

import { GetBalanceParameters, getEthTokenBalance, getVaraAccountBalance } from '../../api';
import GiftSVG from '../../assets/gift.svg?react';
import { TURNSTILE_SITEKEY } from '../../consts';

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
  const { api } = useApi();
  const { account } = useAccount();
  const { isMainnet } = useNetworkType();

  if (isMainnet || !api || !account) return;

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
  const { isMainnet } = useNetworkType();

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
