import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { ComponentProps, useState } from 'react';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { useNetworkType } from '@/context/network-type';
import { TokenPrice } from '@/features/token-price';
import { useAccountsConnection, useEthAccount, useVaraSymbol } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import PlusSVG from '../../assets/plus.svg?react';
import { CLAIM_TYPE, FIELD_NAME, NETWORK, PRIORITY } from '../../consts';
import { useBridgeContext } from '../../context';
import { useSwapForm } from '../../hooks';
import { UseSendTxs, UseAccountBalance, UseFTBalance, UseFee, FormattedValues, UseTxsEstimate } from '../../types';
import { AmountInput } from '../amount-input';
import { Balance } from '../balance';
import { Settings } from '../settings';
import { ConnectWalletButton } from '../submit-button';
import { SubmitProgressBar } from '../submit-progress-bar';
import { SwapNetworkButton } from '../swap-network-button';
import { Token } from '../token';
import { TransactionModal } from '../transaction-modal';
import { WalletAddressButton } from '../wallet-address-button';

import styles from './swap-form.module.scss';

type Props = {
  useAccountBalance: UseAccountBalance;
  useFTBalance: UseFTBalance;
  useSendTxs: UseSendTxs;
  useTxsEstimate: UseTxsEstimate;
  useFee: UseFee;
};

function SwapForm({ useAccountBalance, useFTBalance, useFee, useSendTxs, useTxsEstimate }: Props) {
  const { NETWORK_PRESET } = useNetworkType();
  const { network, token, destinationToken } = useBridgeContext();

  const { api } = useApi();

  const { bridgingFee, vftManagerFee, priorityFee, ...config } = useFee();
  const fees = { bridgingFee, vftManagerFee, priorityFee };
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(token?.address);

  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { isVaraAccount, isEthAccount } = useAccountsConnection();
  const isNetworkAccountConnected = (network.isVara && isVaraAccount) || (!network.isVara && isEthAccount);
  const destinationAccountAddress = network.isVara ? ethAccount.address : account?.address;
  const formattedDestinationAccountAddress = network.isVara
    ? ethAccount.address?.toLowerCase()
    : account?.decodedAddress.toLowerCase();

  const [transactionModal, setTransactionModal] = useState<
    Omit<ComponentProps<typeof TransactionModal>, 'renderProgressBar'> | undefined
  >();

  const [priority, setPriority] = useState<(typeof PRIORITY)[keyof typeof PRIORITY]>(PRIORITY.DEFAULT);
  const shouldPayPriorityFee = priority === PRIORITY.HIGH;
  const time = shouldPayPriorityFee || !network.isVara ? '20 mins' : '1 hour';

  const [claimType, setClaimType] = useState<(typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE]>(CLAIM_TYPE.AUTO);
  const shouldPayBridgingFee = claimType === CLAIM_TYPE.AUTO;

  const varaSymbol = useVaraSymbol();

  const { form, amount, formattedAddress, formattedValues, handleSubmit, setMaxBalance, setAddress } = useSwapForm({
    shouldPayBridgingFee,
    accountBalance: accountBalance.data,
    ftBalance: ftBalance.data,
  });

  const txsEstimate = useTxsEstimate({
    formValues: formattedValues,
    shouldPayBridgingFee,
    shouldPayPriorityFee,
    ftBalance: ftBalance.data,
    ...fees,
  });

  const getContractFees = () => {
    if (isUndefined(bridgingFee)) return;

    let result = 0n;

    if (network.isVara) {
      if (isUndefined(vftManagerFee) || isUndefined(priorityFee)) return;

      result += vftManagerFee;
      if (shouldPayPriorityFee) result += priorityFee;
    }

    if (shouldPayBridgingFee) result += bridgingFee;

    return result;
  };

  const contractFees = getContractFees();

  const getTotalFees = () => {
    if (isUndefined(txsEstimate.data) || isUndefined(contractFees)) return;

    return txsEstimate.data.fees + contractFees;
  };

  const totalFees = getTotalFees();

  const openTransactionModal = (values: FormattedValues) => {
    definedAssert(token, 'Token');
    definedAssert(destinationToken, 'Destination token');
    definedAssert(totalFees, 'Transaction estimation');

    setTransactionModal({
      amount: values.amount,
      receiver: values.accountAddress,
      isVaraNetwork: network.isVara,
      source: token.address,
      destination: destinationToken.address,
      estimatedFees: totalFees,
      time,
      close: () => setTransactionModal(undefined),
    });
  };

  const sendTxs = useSendTxs({
    shouldPayBridgingFee,
    shouldPayPriorityFee,
    ftBalance: ftBalance.data,
    onTransactionStart: openTransactionModal,
    ...fees,
  });

  const isLoading =
    sendTxs.isPending || accountBalance.isLoading || ftBalance.isLoading || config.isLoading || txsEstimate.isLoading;

  const renderFromBalance = () => {
    const balance = token?.isNative ? accountBalance : ftBalance;

    return (
      <Balance
        value={balance.data}
        decimals={token?.decimals}
        symbol={token?.displaySymbol}
        isLoading={balance.isLoading}
        onMaxButtonClick={setMaxBalance}
      />
    );
  };

  const isEnoughBalance = () => {
    if (!api || !token || isUndefined(bridgingFee) || !txsEstimate.data || !accountBalance.data) return false;

    return accountBalance.data > txsEstimate.data.requiredBalance;
  };

  const getButtonText = () => {
    if (!txsEstimate.data) return 'Fill the form';
    if (!isEnoughBalance()) return `Not Enough ${network.isVara ? varaSymbol : 'ETH'}`;

    return 'Transfer';
  };

  const renderTokenPrice = () => <TokenPrice symbol={token?.symbol} amount={amount} className={styles.price} />;
  const renderProgressBar = () => <SubmitProgressBar isVaraNetwork={network.isVara} {...sendTxs} />;

  return (
    <>
      <FormProvider {...form}>
        <form onSubmit={handleSubmit(sendTxs.mutateAsync)} className={styles.form}>
          <div>
            <div className={styles.card}>
              <header className={styles.header}>
                <h3 className={styles.heading}>From</h3>
                <AmountInput.Error />
              </header>

              <div className={styles.row}>
                <Token
                  type="select"
                  address={token?.address}
                  symbol={token?.displaySymbol}
                  networkText={network.isVara ? NETWORK_PRESET.NETWORK_NAME.VARA : NETWORK_PRESET.NETWORK_NAME.ETH}
                  network={network.name}
                />

                <AmountInput />
              </div>

              <div className={styles.balanceFooter}>
                {renderFromBalance()}
                {renderTokenPrice()}
              </div>

              <SwapNetworkButton />
            </div>

            <div className={styles.card}>
              <h3 className={styles.heading}>To</h3>

              <div className={styles.row}>
                <Token
                  type="text"
                  address={destinationToken?.address}
                  symbol={destinationToken?.displaySymbol}
                  networkText={network.isVara ? NETWORK_PRESET.NETWORK_NAME.ETH : NETWORK_PRESET.NETWORK_NAME.VARA}
                  network={network.name === NETWORK.VARA ? NETWORK.ETH : NETWORK.VARA}
                />

                <AmountInput.Value />
              </div>

              <div className={styles.priceFooter}>{renderTokenPrice()}</div>

              <div className={styles.inputContainer}>
                {destinationAccountAddress && (
                  <WalletAddressButton
                    isActive={formattedAddress === formattedDestinationAccountAddress}
                    onClick={() =>
                      setAddress(
                        formattedAddress === formattedDestinationAccountAddress ? '' : destinationAccountAddress,
                      )
                    }
                  />
                )}

                <Input
                  icon={PlusSVG}
                  name={FIELD_NAME.ADDRESS}
                  label="Bridge to"
                  className={styles.input}
                  spellCheck={false}
                  block
                />
              </div>
            </div>
          </div>

          <Settings
            priority={priority}
            claimType={claimType}
            onPriorityChange={setPriority}
            onClaimTypeChange={setClaimType}
            isVaraNetwork={network.isVara}
            fee={totalFees || contractFees}
            isFeeLoading={config.isLoading || txsEstimate.isLoading}
            disabled={isLoading}
            time={time}
          />

          {isNetworkAccountConnected ? (
            <Button type="submit" text={getButtonText()} disabled={!isEnoughBalance()} isLoading={isLoading} block />
          ) : (
            <ConnectWalletButton />
          )}
        </form>
      </FormProvider>

      {/* passing renderProgressBar explicitly to avoid state closure */}
      {transactionModal && <TransactionModal renderProgressBar={renderProgressBar} {...transactionModal} />}
    </>
  );
}

export { SwapForm };
