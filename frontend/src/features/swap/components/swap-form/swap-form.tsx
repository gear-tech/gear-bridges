import { useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { NETWORK_NAME } from '@/consts';

import GasSVG from '../../assets/gas.svg?react';
import { FIELD_NAME } from '../../consts';
import { useSwapForm, useBridge, useFeeCalculator } from '../../hooks';
import { NetworkName, UseBalance, UseConfig, UseHandleSubmit } from '../../types';
import { getNormalizedTime } from '../../utils';
import { Balance } from '../balance';
import { FeeLoader } from '../fee-loader';
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
  const alert = useAlert();

  const { feeCalculatorData, isFeeLoading, refetch, error } = useFeeCalculator({ networkName });
  const { fee, mortality, timestamp } = feeCalculatorData || {};

  const FromNetwork = isVaraNetwork ? Network.Vara : Network.Eth;
  const ToNetwork = isVaraNetwork ? Network.Eth : Network.Vara;

  const { contract, options, symbol, pair } = useBridge(networkName);

  const config = useConfig(contract);
  const { minValue } = config;

  const balance = useBalance(config);

  const { onSubmit, isSubmitting } = useHandleSubmit(contract, config, feeCalculatorData);

  const { form, onValueChange, onExpectedValueChange, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    balance,
    BigInt(0),
    minValue,
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

  const onTimeEnd = () => {
    refetch().catch(() => {
      alert.error('Unable to fetch fee data');
    });
  };

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
          <div className={styles.balance}>
            <Balance
              SVG={GasSVG}
              heading="Expected Fee"
              value={fee?.formattedValue}
              isLoading={config.isLoading}
              unit={symbol.native}
            />
            <FeeLoader
              startTimestamp={getNormalizedTime(networkName, timestamp)}
              mortality={getNormalizedTime(networkName, mortality)}
              onTimeEnd={onTimeEnd}
            />
          </div>

          <Button
            type="submit"
            text="Swap"
            disabled={disabled || !!error}
            isLoading={isSubmitting || balance.isLoading || config.isLoading || isFeeLoading}
          />
        </footer>
      </form>
    </FormProvider>
  );
}

export { SwapForm };
