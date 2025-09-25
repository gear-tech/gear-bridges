import { Logger } from '@subsquid/logger';

import { EthBridgeMessageQueuedEvent, MessageQueuedEvent, ProgramChangedEvent, UserMessageSentEvent } from './event';
import { BatchState } from '../batch-state';
import { Block } from '../processor';
import { Decoder } from '../codec';
import { RpcClient } from '@subsquid/rpc-client';

interface CommonHandlerContext {
  readonly state: BatchState;
  readonly log: Logger;
  readonly blockHeader: Block;
}

export interface UserMessageSentHandlerContext extends CommonHandlerContext {
  readonly service: string;
  readonly method: string;
  readonly decoder: Decoder;
  readonly event: UserMessageSentEvent;
}

export interface MessageQueuedContext extends CommonHandlerContext {
  readonly decoder: Decoder;
  readonly event: MessageQueuedEvent;
}

export interface EthBridgeMessageQueuedContext extends CommonHandlerContext {
  readonly event: EthBridgeMessageQueuedEvent;
}

export interface ProgramChangedHandlerContext extends CommonHandlerContext {
  event: ProgramChangedEvent;
  rpc: RpcClient;
}
