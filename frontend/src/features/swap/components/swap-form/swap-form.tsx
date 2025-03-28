import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { ComponentProps, useState, JSX } from 'react';
import { FormProvider } from 'react-hook-form';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { Input, Skeleton } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { TransactionModal } from '@/features/history/components/transaction-modal';
import { Network as TransferNetwork } from '@/features/history/types';
import { useEthAccount } from '@/hooks';
import { isUndefined } from '@/utils';

import PlusSVG from '../../assets/plus.svg?react';
import { FIELD_NAME, NETWORK_INDEX } from '../../consts';
import { useSwapForm, useToken } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance } from '../../types';
import { getMergedBalance } from '../../utils';
import { AmountInput } from '../amount-input';
import { Balance } from '../balance';
import { DetailsAccordion } from '../details-accordion';
import { FTAllowanceTip } from '../ft-allowance-tip';
import { SelectToken } from '../select-token';
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

  const { api, isApiReady } = useApi();
  const [pairIndex, setPairIndex] = useState(0);
  const { address, destinationAddress, destinationSymbol, symbol, decimals, ...bridge } = useToken(
    networkIndex,
    pairIndex,
  );
  const isNativeToken = address === WRAPPED_VARA_CONTRACT_ADDRESS;

  const { fee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(address);
  const allowance = useFTAllowance(address);

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

  const [submit, approve, mint] = useHandleSubmit(
    address,
    fee.value,
    allowance.data,
    ftBalance.data,
    accountBalance.data,
    openTransacionModal,
  );

  const { form, amount, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    isNativeToken,
    accountBalance,
    ftBalance,
    decimals,
    disabled,
    submit.mutateAsync,
  );

  const renderFromBalance = () => {
    const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance) : ftBalance;

    return (
      <Balance
        value={balance.data}
        decimals={decimals}
        symbol={symbol}
        isLoading={balance.isLoading || bridge.isLoading}
        onMaxButtonClick={setMaxBalance}
      />
    );
  };

  const isBalanceValid = () => {
    if (!isApiReady || accountBalance.isLoading || config.isLoading) return true; // not valid ofc, but we don't want to render error
    if (!accountBalance.data || isUndefined(fee.value)) return false;

    const requiredBalance = isVaraNetwork ? fee.value + api.existentialDeposit.toBigInt() : fee.value;

    return accountBalance.data > requiredBalance;
  };

  const getButtonText = () => {
    if (!isBalanceValid()) return isVaraNetwork ? 'Not enough VARA' : 'Not enough ETH';

    if (mint?.isPending) return 'Locking...';
    if (approve.isPending) return 'Approving...';
    if (submit.isPending) return 'Swapping...';

    return 'Transfer';
  };

  const renderProgressBar = () => <SubmitProgressBar mint={mint} approve={approve} submit={submit} />;

  return (
    <FormProvider {...form}>
      <form onSubmit={handleSubmit} className={styles.form}>
        <div>
          <div className={styles.card}>
            <header className={styles.header}>
              <h3 className={styles.heading}>From</h3>
              <AmountInput.Error />
            </header>

            <div className={styles.row}>
              <div className={styles.wallet}>
                {isVaraNetwork ? <VaraSVG className={styles.networkIcon} /> : <EthSVG className={styles.networkIcon} />}

                <div className={styles.token}>
                  <SelectToken
                    pairIndex={pairIndex}
                    isVaraNetwork={isVaraNetwork}
                    symbol={symbol}
                    accountBalance={accountBalance}
                    onChange={setPairIndex}
                  />

                  <p className={styles.network}>{isVaraNetwork ? 'Vara' : 'Ethereum'}</p>
                </div>
              </div>

              <AmountInput />
            </div>

            {renderFromBalance()}
            {renderSwapNetworkButton()}
          </div>

          <div className={styles.card}>
            <h3 className={styles.heading}>To</h3>

            <div className={styles.toContainer}>
              <div className={styles.wallet}>
                {isVaraNetwork ? <EthSVG className={styles.networkIcon} /> : <VaraSVG className={styles.networkIcon} />}

                <div className={styles.token}>
                  <p className={styles.symbol}>{destinationSymbol || <Skeleton width="6rem" />}</p>
                  <p className={styles.network}>{isVaraNetwork ? 'Ethereum' : 'Vara'}</p>
                </div>
              </div>

              <AmountInput.Value decimals={decimals} />
            </div>

            <div className={styles.inputContainer}>
              <Input
                icon={PlusSVG}
                name={FIELD_NAME.ADDRESS}
                label={isVaraNetwork ? 'ERC20 Address' : 'Substrate Address'}
                className={styles.input}
                spellCheck={false}
                block
              />
            </div>
          </div>
        </div>

        <DetailsAccordion fee={fee.formattedValue} symbol={isVaraNetwork ? 'VARA' : 'ETH'} />

        <footer className={styles.submitContainer}>
          <Button
            type="submit"
            text={getButtonText()}
            disabled={disabled || !isBalanceValid()}
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
