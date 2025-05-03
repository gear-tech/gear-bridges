import { HexString } from '@gear-js/api';
import { useAccount, useAlert, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import HCaptcha from '@hcaptcha/react-hcaptcha';
import { useMutation } from '@tanstack/react-query';
import { useRef } from 'react';

import { LinkButton } from '@/components';
import { ETH_WRAPPED_ETH_CONTRACT_ADDRESS, USDC_CONTRACT_ADDRESS, USDT_CONTRACT_ADDRESS } from '@/consts/env';
import { useEthAccount } from '@/hooks';

import { GetBalanceParameters, getEthTokenBalance, getVaraAccountBalance } from '../../api';
import GiftSVG from '../../assets/gift.svg?react';
import { HCAPTCHA_SITEKEY } from '../../consts';

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
  const hCaptchaRef = useRef<HCaptcha>(null);

  const { mutateAsync, isPending } = useMutation({
    mutationFn: async () => {
      if (!hCaptchaRef.current) throw new Error('HCaptcha ref is null');

      const token = (await hCaptchaRef.current.execute({ async: true })).response;
      const payload = parameters as T;

      return getBalance({ token, payload });
    },
  });

  const handleClick = () =>
    mutateAsync()
      .then(() => {
        onSuccess?.();
        alert.success('Tokens sent to your address');
      })
      .catch((error: string | Error) => {
        if (error === 'challenge-closed') return;

        alert.error(error instanceof Error ? error.message : error);
      });

  return (
    <div>
      <Button onClick={handleClick} isLoading={isPending} {...BUTTON_PROPS} />
      <HCaptcha ref={hCaptchaRef} theme="dark" size="invisible" sitekey={HCAPTCHA_SITEKEY} />
    </div>
  );
}

function GetVaraAccountBalanceButton() {
  const { api } = useApi();
  const { account } = useAccount();

  if (!account || !api) return;

  return (
    <ButtonComponent getBalance={getVaraAccountBalance} address={account.address} genesis={api.genesisHash.toHex()} />
  );
}

function GetEthTokenBalanceButton({ address, onSuccess }: { address: HexString; onSuccess: () => void }) {
  const ethAccount = useEthAccount();

  if (!ethAccount.address) return;

  if (address === ETH_WRAPPED_ETH_CONTRACT_ADDRESS)
    return <LinkButton type="external" to="https://holesky-faucet.pk910.de/" {...BUTTON_PROPS} />;

  if (address !== USDC_CONTRACT_ADDRESS && address !== USDT_CONTRACT_ADDRESS) return;

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
