import {
  EthBridgeMessageQueuedEvent,
  MessageQueuedEvent,
  ProgramChangedEvent,
  QueueMerkleRootChangedEvent,
  UserMessageSentEvent,
} from './types/index.js';
import { Event } from './processor.js';

export function isMessageQueued(event: Event): event is MessageQueuedEvent {
  return event.name === 'Gear.MessageQueued';
}

export function isUserMessageSent(event: Event): event is UserMessageSentEvent {
  return event.name === 'Gear.UserMessageSent';
}

export function isProgramChanged(event: Event): event is ProgramChangedEvent {
  return event.name === 'Gear.ProgramChanged';
}

export function isEthBridgeMessageQueued(event: Event): event is EthBridgeMessageQueuedEvent {
  return event.name === 'GearEthBridge.MessageQueued';
}

export function isQueueMerkleRootChanged(event: Event): event is QueueMerkleRootChangedEvent {
  return event.name === 'GearEthBridge.QueueMerkleRootChanged';
}

export function isQueueReset(event: Event): boolean {
  return event.name === 'GearEthBridge.QueueReset';
}

export const enum ProgramName {
  VftManager = 'vft_manager',
  HistoricalProxy = 'historical_proxy',
  BridgingPayment = 'bridging_payment',
  CheckpointClient = 'checkpoint_client',
}

export const enum VftManagerServices {
  VftManager = 'VftManager',
}

export const enum VftManagerMethods {
  BridgingRequested = 'BridgingRequested',
  TokenMappingAdded = 'TokenMappingAdded',
  TokenMappingRemoved = 'TokenMappingRemoved',
  HistoricalProxyAddressChanged = 'HistoricalProxyAddressChanged',
  RequestBridging = 'RequestBridging',
}

export const enum HistoricalProxyServices {
  HistoricalProxy = 'HistoricalProxy',
}

export const enum HistoricalProxyMethods {
  Relayed = 'Relayed',
}

export const enum BridgingPaymentServices {
  BridgingPayment = 'BridgingPayment',
}

export const enum BridgingPaymentMethods {
  BridgingPaid = 'BridgingPaid',
  PriorityBridgingPaid = 'PriorityBridgingPaid',
}

export const enum CheckpointClientServices {
  ServiceSyncUpdate = 'ServiceSyncUpdate',
  ServiceReplayBack = 'ServiceReplayBack',
}

export const enum CheckpointClientMethods {
  NewCheckpoint = 'NewCheckpoint',
}
