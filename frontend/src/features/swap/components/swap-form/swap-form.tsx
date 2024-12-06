import { useAccount } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { ComponentProps, useState } from 'react';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { TransactionModal } from '@/features/history/components/transaction-modal';
import { Network as TransferNetwork } from '@/features/history/types';
import { useEthAccount } from '@/hooks';

import GasSVG from '../../assets/gas.svg?react';
import { FIELD_NAME, NETWORK_INDEX } from '../../consts';
import { useSwapForm, useBridge } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance } from '../../types';
import { getMergedBalance } from '../../utils';
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

  const { address, destinationAddress, options, symbol, pair, decimals, ...bridge } = useBridge(networkIndex);
  const isNativeToken = address === WRAPPED_VARA_CONTRACT_ADDRESS;

  const { fee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(address, decimals);
  const allowance = useFTAllowance(address);

  const [{ mutateAsync: onSubmit, ...submit }, approve, mint] = useHandleSubmit(
    address,
    fee.value,
    allowance.data,
    ftBalance.value,
  );

  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const [transactionModal, setTransactionModal] = useState<
    Omit<ComponentProps<typeof TransactionModal>, 'close'> | undefined
  >();

  const getSubmitStatus = () => {
    if (mint?.isPending) return 'mint';
    if (approve.isPending) return 'approve';
    if (submit.isPending) return 'transfer';
  };

  const openTransacionModal = (amount: string, receiver: string) => {
    if (!address || !destinationAddress) throw new Error('Address is not defined');

    const source = address;
    const destination = destinationAddress;
    const sourceNetwork = isVaraNetwork ? TransferNetwork.Gear : TransferNetwork.Ethereum;
    const destNetwork = isVaraNetwork ? TransferNetwork.Ethereum : TransferNetwork.Gear;
    const sender = isVaraNetwork ? account!.decodedAddress : ethAccount.address!;

    setTransactionModal({ amount, source, destination, sourceNetwork, destNetwork, sender, receiver });
  };

  const closeTransactionModal = () => setTransactionModal(undefined);

  const { form, amount, onValueChange, onExpectedValueChange, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    isNativeToken,
    accountBalance,
    ftBalance,
    decimals,
    fee.value,
    disabled,
    onSubmit,
    openTransacionModal,
    closeTransactionModal,
  );

  const renderFromBalance = () => {
    const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance, decimals) : ftBalance;

    return (
      <Balance
        value={balance.formattedValue}
        unit={symbol}
        isLoading={balance.isLoading || bridge.isLoading}
        onMaxButtonClick={setMaxBalance}
      />
    );
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
              text="Transfer"
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

      {transactionModal && (
        <TransactionModal close={closeTransactionModal} {...transactionModal} loadingStatus={getSubmitStatus()} />
      )}
    </FormProvider>
  );
}

export { SwapForm };
