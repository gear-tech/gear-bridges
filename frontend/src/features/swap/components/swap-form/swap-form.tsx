import { Button } from '@gear-js/vara-ui';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';

import GasSVG from '../../assets/gas.svg?react';
import { FIELD_NAME, NETWORK_INDEX } from '../../consts';
import { useSwapForm, useBridge, useVaraConfig } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance } from '../../types';
import { Balance } from '../balance';
import { Network } from '../network';

import styles from './swap-form.module.scss';

type Props = {
  networkIndex: number;
  disabled: boolean;
  useAccountBalance: UseAccountBalance;
  useFTBalance: UseFTBalance;
  useHandleSubmit: UseHandleSubmit;
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapForm({
  networkIndex,
  disabled,
  useHandleSubmit,
  useAccountBalance,
  useFTBalance,
  renderSwapNetworkButton,
}: Props) {
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;
  const FromNetwork = isVaraNetwork ? Network.Vara : Network.Eth;
  const ToNetwork = isVaraNetwork ? Network.Eth : Network.Vara;

  const { address, options, symbol, pair,  ...bridge } = useBridge(networkIndex);
  const { fee, ...config } = useVaraConfig(isVaraNetwork);
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(address);
  const { onSubmit, isSubmitting } = useHandleSubmit(address, fee.value);

  const { form, onValueChange, onExpectedValueChange, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    accountBalance,
    ftBalance,
    fee.value,
    disabled,
    onSubmit,
  );

  const renderFromBalance = () => (
    <Balance
      value={ftBalance.formattedValue}
      unit={symbol}
      isLoading={ftBalance.isLoading || bridge.isLoading}
      onMaxButtonClick={setMaxBalance}
    />
  );

  return (
    <FormProvider {...form}>
      <form className={styles.form} onSubmit={handleSubmit}>
        <div className={styles.section}>
          <FromNetwork
            options={options.from}
            selectValue={pair.value}
            inputName={FIELD_NAME.VALUE}
            onSelectChange={pair.set}
            onChange={onValueChange}
            renderBalance={renderFromBalance}
          />

          {renderSwapNetworkButton()}
        </div>

        <div className={styles.section}>
          <Input name={FIELD_NAME.ADDRESS} label={isVaraNetwork ? 'To ERC20 address' : 'To Substrate address'} block />

          <ToNetwork
            options={options.to}
            selectValue={pair.value}
            inputName={FIELD_NAME.EXPECTED_VALUE}
            onSelectChange={pair.set}
            onChange={onExpectedValueChange}
          />
        </div>

        <footer className={styles.footer}>
          <Balance
            SVG={GasSVG}
            heading="Expected Fee"
            value={fee.formattedValue}
            isLoading={config.isLoading}
            unit={isVaraNetwork ? 'VARA' : 'ETH'}
          />

          <Button
            type="submit"
            text="Swap"
            disabled={disabled}
            isLoading={
              isSubmitting || accountBalance.isLoading || ftBalance.isLoading || config.isLoading || bridge.isLoading
            }
          />
        </footer>
      </form>
    </FormProvider>
  );
}

export { SwapForm };
