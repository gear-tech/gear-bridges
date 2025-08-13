import { Event } from './processor';
import { EthBridgeMessageQueuedEvent, MessageQueuedEvent, UserMessageSentEvent } from './types';

export function isMessageQueued(event: Event): event is MessageQueuedEvent {
  return event.name === 'Gear.MessageQueued';
}

export function isUserMessageSent(event: Event): event is UserMessageSentEvent {
  return event.name === 'Gear.UserMessageSent';
}

export function isProgramChanged(event: Event): boolean {
  return event.name === 'Gear.ProgramChanged';
}

export function isEthBridgeMessageQueued(event: Event): event is EthBridgeMessageQueuedEvent {
  return event.name === 'GearEthBridge.MessageQueued';
}

export const enum ProgramName {
  VftManager = 'vft_manager',
  HistoricalProxy = 'historical_proxy',
  BridgingPayment = 'bridging_payment',
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
}
