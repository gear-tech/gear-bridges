import { Input, Select, SelectProps } from '@gear-js/vara-ui';
import { ReactNode } from 'react';
import { Controller } from 'react-hook-form';
import { NumericFormat } from 'react-number-format';
import { SourceType } from 'react-number-format/types/types';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { NetworkCard } from '@/components';
import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import { FIELD_NAME } from '../../consts';

import styles from './network.module.scss';

type Props = {
  name: string;
  SVG: SVGComponent;
  options: SelectProps['options'];
  selectValue: string;
  inputName: typeof FIELD_NAME.VALUE | typeof FIELD_NAME.EXPECTED_VALUE;
  onChange: (value: string) => void;
  onSelectChange: (value: string) => void;
  renderBalance?: () => ReactNode;
};

function Network({
  name,
  SVG,
  options,
  selectValue,
  inputName,
  onChange,
  onSelectChange,
  renderBalance = () => <></>,
}: Props) {
  return (
    <div className={cx(styles.network, styles[inputName])}>
      <NetworkCard SVG={SVG} name={name} />

      <div className={styles.inputs}>
        <Select options={options} value={selectValue} onChange={({ target }) => onSelectChange(target.value)} />

        <div className={styles.input}>
          <Controller
            name={inputName}
            render={({ field, fieldState: { error } }) => (
              <NumericFormat
                value={field.value as string}
                onValueChange={({ value }, { source }) => {
                  // seems like onValueChange is triggered when input value formatting changes too,
                  // therefore resulting in twice onChange calls in our case of two inputs that depend on each other.
                  // skipping this behaviour to rely only on user's input.
                  if (source === ('prop' as SourceType)) return; // assertion cuz of enum import error

                  field.onChange(value);
                  onChange(value);
                }}
                customInput={Input}
                label={field.name === FIELD_NAME.VALUE ? 'Value' : 'Expected value'}
                error={error?.message}
                allowNegative={false}
                thousandSeparator
                block
              />
            )}
          />

          {renderBalance()}
        </div>
      </div>
    </div>
  );
}

function VaraNetwork(props: Omit<Props, 'SVG' | 'name'>) {
  return <Network SVG={VaraSVG} name="Vara" {...props} />;
}

function EthNetwork(props: Omit<Props, 'SVG' | 'name'>) {
  return <Network SVG={EthSVG} name="Ethereum" {...props} />;
}

Network.Vara = VaraNetwork;
Network.Eth = EthNetwork;

export { Network };
