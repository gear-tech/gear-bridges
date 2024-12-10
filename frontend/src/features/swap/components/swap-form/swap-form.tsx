import { useAccount } from '@gear-js/react-hooks';
import { Button, Select } from '@gear-js/vara-ui';
import { ComponentProps, useState } from 'react';
import { FormProvider } from 'react-hook-form';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FeeAndTimeFooter, Input, Skeleton } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { TransactionModal } from '@/features/history/components/transaction-modal';
import { Network as TransferNetwork } from '@/features/history/types';
import { NetworkWalletField } from '@/features/wallet';
import { useEthAccount } from '@/hooks';
import { cx } from '@/utils';

import WalletSVG from '../../assets/wallet.svg?react';
import { FIELD_NAME, NETWORK_INDEX } from '../../consts';
import { useSwapForm, useBridge } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance } from '../../types';
import { getMergedBalance } from '../../utils';
import { AmountInput } from '../amount-input';
import { Balance } from '../balance';
import { FTAllowanceTip } from '../ft-allowance-tip';
import { NetworkCard } from '../network-card';
import { SubmitProgressBar } from '../submit-progress-bar';

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
  const [transactionModal, setTransactionModal] = useState<ComponentProps<typeof TransactionModal> | undefined>();

  const openTransacionModal = (amount: string, receiver: string) => {
    if (!address || !destinationAddress) throw new Error('Address is not defined');

    const source = address;
    const destination = destinationAddress;
    const sourceNetwork = isVaraNetwork ? TransferNetwork.Gear : TransferNetwork.Ethereum;
    const destNetwork = isVaraNetwork ? TransferNetwork.Ethereum : TransferNetwork.Gear;
    const sender = isVaraNetwork ? account!.decodedAddress : ethAccount.address!;
    const close = () => setTransactionModal(undefined);

    setTransactionModal({ amount, source, destination, sourceNetwork, destNetwork, sender, receiver, close });
  };

  const { form, amount, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    isNativeToken,
    accountBalance,
    ftBalance,
    decimals,
    fee.value,
    disabled,
    onSubmit,
    openTransacionModal,
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

  const getButtonText = () => {
    if (mint?.isPending) return 'Locking...';
    if (approve.isPending) return 'Approving...';
    if (submit.isPending) return 'Swapping...';

    return 'Transfer';
  };

  const renderProgressBar = () => <SubmitProgressBar mint={mint} approve={approve} submit={submit} />;

  return (
    <FormProvider {...form}>
      <form onSubmit={handleSubmit}>
        <div className={styles.sections}>
          <div className={cx(styles.section, !disabled && styles.active)}>
            <div className={styles.row}>
              <div className={styles.wallet}>
                <NetworkCard
                  destination="From"
                  SVG={isVaraNetwork ? VaraSVG : EthSVG}
                  name={isVaraNetwork ? 'Vara' : 'Ethereum'}
                />

                <NetworkWalletField />
              </div>

              {accountBalance.formattedValue && (
                <div className={styles.balance}>
                  <WalletSVG />
                  {`${accountBalance.formattedValue} ${isVaraNetwork ? 'VARA' : 'ETH'}`}
                </div>
              )}

              {accountBalance.isLoading && <Skeleton />}
            </div>

            <div className={styles.row}>
              <div className={styles.amount}>
                <AmountInput />

                <Select
                  options={options}
                  value={pair.value}
                  onChange={({ target }) => pair.set(target.value)}
                  className={styles.select}
                  disabled={options.length === 0}
                />
              </div>

              {renderFromBalance()}
            </div>

            {renderSwapNetworkButton()}
          </div>

          <div className={cx(styles.section, !disabled && styles.active)}>
            <div className={styles.row}>
              <div className={styles.destination}>
                <NetworkCard
                  destination="To"
                  SVG={isVaraNetwork ? EthSVG : VaraSVG}
                  name={isVaraNetwork ? 'Ethereum' : 'Vara'}
                />

                <Input
                  name={FIELD_NAME.ADDRESS}
                  label={isVaraNetwork ? 'To ERC20 address' : 'To Substrate address'}
                  block
                />
              </div>

              <Balance heading="Receive" value={amount || '0'} unit={symbol} />
            </div>

            <FeeAndTimeFooter fee={fee.formattedValue} symbol={isVaraNetwork ? 'VARA' : 'ETH'} />
          </div>
        </div>

        <footer className={styles.submitContainer}>
          <Button
            type="submit"
            text={getButtonText()}
            size="small"
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
        </footer>
      </form>

      {transactionModal && <TransactionModal renderProgressBar={renderProgressBar} {...transactionModal} />}
    </FormProvider>
  );
}

export { SwapForm };
