import { Button } from '@gear-js/vara-ui';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';

import GasSVG from '../../assets/gas.svg?react';
import { FIELD_NAME, NETWORK_INDEX } from '../../consts';
import { useSwapForm, useBridge } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance } from '../../types';
import { Balance } from '../balance';
import { FTAllowanceTip } from '../ft-allowance-tip';
import { Network } from '../network';

import styles from './swap-form.module.scss';

type Props = {
  networkIndex: number;
  disabled: boolean;
  useAccountBalance: UseAccountBalance;
  useFTBalance: UseFTBalance;
  useFTAllowance: UseFTAllowance;
  useHandleSubmit: UseHandleSubmit;
  useFee: UseFee;
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapForm({
  networkIndex,
  disabled,
  useHandleSubmit,
  useAccountBalance,
  useFTBalance,
  useFTAllowance,
  useFee,
  renderSwapNetworkButton,
}: Props) {
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;
  const FromNetwork = isVaraNetwork ? Network.Vara : Network.Eth;
  const ToNetwork = isVaraNetwork ? Network.Eth : Network.Vara;

  const { address, options, symbol, pair, decimals, ...bridge } = useBridge(networkIndex);
  const { fee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(address, decimals);
  const allowance = useFTAllowance(address);
  const [{ mutateAsync: onSubmit, ...submit }, approve] = useHandleSubmit(address, fee.value, allowance.data);

  const { form, amount, onValueChange, onExpectedValueChange, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    accountBalance,
    ftBalance,
    decimals,
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

  const getButtonText = () => {
    if (approve.isPending) return 'Approving...';
    if (submit.isPending) return 'Swapping...';

    return 'Swap';
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
          <Balance
            SVG={GasSVG}
            heading="Expected Fee"
            value={fee.formattedValue}
            isLoading={config.isLoading}
            unit={isVaraNetwork ? 'VARA' : 'ETH'}
          />

          <div className={styles.submitContainer}>
            <Button
              type="submit"
              text={getButtonText()}
              disabled={disabled}
              isLoading={
                approve.isLoading ||
                submit.isPending ||
                accountBalance.isLoading ||
                ftBalance.isLoading ||
                config.isLoading ||
                bridge.isLoading ||
                allowance.isLoading
              }
              block
            />

            <FTAllowanceTip
              allowance={allowance.data}
              decimals={decimals}
              symbol={symbol}
              amount={amount}
              isVaraNetwork={isVaraNetwork}
              isLoading={bridge.isLoading || allowance.isLoading}
            />
          </div>
        </footer>
      </form>
    </FormProvider>
  );
}

export { SwapForm };
