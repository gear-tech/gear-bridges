import { HexString } from '@gear-js/api';
import { getSlotByBlockNumber } from '@gear-js/bridge';
import { useAlert, useProgram, useProgramQuery } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { useEffect } from 'react';
import { usePublicClient } from 'wagmi';

import { isUndefined, logger } from '@/utils';

import { CONTRACT_ADDRESS, ETH_BEACON_NODE_ADDRESS } from '../../consts';

import { CheckpointClient } from './check-client';
import { EthEventsClient } from './eth-events';
import { HistoricalProxyClient } from './hist-proxy';

function useSlot(blockNumber: bigint) {
  const alert = useAlert();
  const publicClient = usePublicClient();

  const query = useQuery({
    queryKey: ['slotByBlockNumber', blockNumber.toString()],
    queryFn: () => getSlotByBlockNumber(ETH_BEACON_NODE_ADDRESS, publicClient!, blockNumber),
    enabled: Boolean(publicClient),
  });

  const { error } = query;

  useEffect(() => {
    if (!error) return;

    logger.error('Get slot by block number', error);
    alert.error(error.message);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [error]);

  return query;
}

function useEthEventsContractAddress(slot: number | undefined) {
  const { data: historicalProxyProgram } = useProgram({
    id: CONTRACT_ADDRESS.HISTORICAL_PROXY,
    library: HistoricalProxyClient,
  });

  return useProgramQuery({
    program: historicalProxyProgram,
    serviceName: 'historicalProxy',
    functionName: 'endpointFor',
    args: [slot!],
    query: { enabled: !isUndefined(slot), select: (data) => ('ok' in data ? data.ok : undefined) },
  });
}

function useCheckpointClientContractAddress(ethEventsContractAddress: HexString | undefined) {
  const { data: ethEventsProgram } = useProgram({
    id: ethEventsContractAddress,
    library: EthEventsClient,
  });

  return useProgramQuery({
    program: ethEventsProgram,
    serviceName: 'ethereumEventClient',
    functionName: 'checkpointLightClientAddress',
    args: [],
  });
}

function useIsEthRelayAvailable(blockNumber: bigint) {
  const { data: slot } = useSlot(blockNumber);
  const { data: ethEventsContractAddress } = useEthEventsContractAddress(slot);
  const { data: checkpointClientContractAddress } = useCheckpointClientContractAddress(ethEventsContractAddress);

  const { data: checkpointClientProgram } = useProgram({
    id: checkpointClientContractAddress,
    library: CheckpointClient,
  });

  return useProgramQuery({
    program: checkpointClientProgram,
    serviceName: 'serviceCheckpointFor',
    functionName: 'get',
    args: [slot!],
    query: { enabled: !isUndefined(slot), select: (data) => 'ok' in data },
  });
}

export { useIsEthRelayAvailable };
