import { Button } from '@gear-js/vara-ui';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { NETWORK_NAME } from '@/consts';

import GasSVG from '../../assets/gas.svg?react';
import { FIELD_NAME } from '../../consts';
import { useSwapForm, useBridge } from '../../hooks';
import { NetworkName, UseBalance, UseConfig, UseHandleSubmit } from '../../types';
import { Balance } from '../balance';
import { Network } from '../network';

import styles from './swap-form.module.scss';

type Props = {
  networkName: NetworkName;
  disabled: boolean;
  useBalance: UseBalance;
  useConfig: UseConfig;
  useHandleSubmit: UseHandleSubmit;
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapForm({ networkName, disabled, useHandleSubmit, useBalance, useConfig, renderSwapNetworkButton }: Props) {
  // TODO: isVaraNetwork and isNativeToken can be use explicitly in some of the hooks
  const isVaraNetwork = networkName === NETWORK_NAME.VARA;

  const FromNetwork = isVaraNetwork ? Network.Vara : Network.Eth;
  const ToNetwork = isVaraNetwork ? Network.Eth : Network.Vara;

  const { contract, options, symbol, pair } = useBridge(networkName);

  const config = useConfig(contract?.address);

  const balance = useBalance('0x00', false);

  const { onSubmit, isSubmitting } = useHandleSubmit(contract, '0x00');

  const { form, onValueChange, onExpectedValueChange, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    balance,
    BigInt(0),
    BigInt(0),
    disabled,
    onSubmit,
  );

  const renderFromBalance = () => (
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
          <Balance SVG={GasSVG} heading="Expected Fee" value={'0'} isLoading={config.isLoading} unit={symbol.native} />

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
