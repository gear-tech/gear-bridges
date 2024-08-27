import { Button } from '@gear-js/vara-ui';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';

import GasSVG from '../../assets/gas.svg?react';
import { FIELD_NAME, NETWORK_INDEX } from '../../consts';
import { useSwapForm, useBridge, useVaraConfig } from '../../hooks';
import { UseBalance, UseHandleSubmit } from '../../types';
import { Balance } from '../balance';
import { Network } from '../network';

import styles from './swap-form.module.scss';

type Props = {
  networkIndex: number;
  disabled: boolean;
  useBalance: UseBalance;
  useHandleSubmit: UseHandleSubmit;
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapForm({ networkIndex, disabled, useHandleSubmit, useBalance, renderSwapNetworkButton }: Props) {
  // TODO: isVaraNetwork and isNativeToken can be use explicitly in some of the hooks
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;
  const FromNetwork = isVaraNetwork ? Network.Vara : Network.Eth;
  const ToNetwork = isVaraNetwork ? Network.Eth : Network.Vara;

  const { address, options, symbol, pair } = useBridge(networkIndex);
  const config = useVaraConfig(isVaraNetwork);
  const balance = useBalance(address, false);
  const { onSubmit, isSubmitting } = useHandleSubmit(address, '0x00');

  const { form, onValueChange, onExpectedValueChange, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    balance,
    BigInt(0),
    BigInt(0),
    disabled,
    onSubmit,
  );

  const renderFromBalance = () =>
    symbol.value && (
      <Balance
        value={balance.formattedValue}
        unit={symbol.value}
        isLoading={balance.isLoading}
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
            value={config.fee.formattedValue}
            isLoading={config.isLoading}
            unit={symbol.native}
          />

          <Button
            type="submit"
            text="Swap"
            disabled={disabled}
            isLoading={isSubmitting || balance.isLoading || config.isLoading}
          />
        </footer>
      </form>
    </FormProvider>
  );
}

export { SwapForm };
