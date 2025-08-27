import { Account, PublicClient, WalletClient } from 'viem';
import { GearApi, HexString } from '@gear-js/api';

import { getMessageQueueClient } from '../ethereum/index.js';
import { initLogger, logger } from '../util.js';
import { GearClient } from '../vara/index.js';
import { messageHash } from './util.js';

/**
 * Relays a message from Vara network to Ethereum by finding the queued message,
 * fetching merkle proof, and processing it through the message queue contract.
 *
 * @param nonce - The message nonce to relay (little endian encoded if hex string)
 * @param blockNumber - The Vara block number containing the message sent by EthBridge builtin
 * @param ethereumPublicClient - Ethereum public client for reading blockchain state
 * @param ethereumWalletClient - Ethereum wallet client for sending transactions
 * @param ethereumAccount - Ethereum account to use for transactions
 * @param gearApi - Gear API instance for interacting with Vara network
 * @param messageQueueuAddress - Ethereum message queue contract address
 * @param silent - Whether to suppress logging output
 * @returns Promise that resolves when the message is successfully processed
 */
export async function relayVaraToEth(
  nonce: bigint | HexString,
  blockNumber: bigint,
  ethereumPublicClient: PublicClient,
  ethereumWalletClient: WalletClient,
  ethereumAccount: Account,
  gearApi: GearApi,
  messageQueueuAddress: `0x${string}`,
  silent = true,
) {
  initLogger(silent);
  const gearClient = new GearClient(gearApi);
  const msgQClient = getMessageQueueClient(
    messageQueueuAddress,
    ethereumPublicClient,
    ethereumWalletClient,
    ethereumAccount,
  );

  let blockHash = (await gearApi.blocks.getBlockHash(Number(blockNumber))).toHex();

  if (typeof nonce === 'string') {
    nonce = BigInt(nonce);
  }

  logger.info(`Fetching message with nonce ${nonce} from block ${blockNumber}`);

  const msg = await gearClient.findMessageQueuedEvent(blockHash, nonce);

  if (!msg) {
    throw new Error(`Message with nonce ${nonce} not found in block ${blockNumber}`);
  }

  const authoritySetId = await gearClient.getAuthoritySetIdByBlockNumber(blockNumber);
  logger.info(`Authority set ID for block ${blockNumber}: ${authoritySetId}`);

  logger.info(`Fetching Merkle root for block ${blockNumber}`);
  let merkleRoot = await msgQClient.getMerkleRoot(blockNumber);

  if (merkleRoot) {
    logger.info(`Received merkle root ${merkleRoot} for block ${blockNumber}`);
  } else {
    const gearBlockTimestamp = (await gearApi.blocks.getBlockTimestamp(blockHash)).toNumber();
    const ethereumHead = await ethereumPublicClient.getBlock();
    const ethereumHeadTimestamp = Number(ethereumHead.timestamp);

    const diff = ethereumHeadTimestamp - gearBlockTimestamp / 1000;

    const [from, to] = [ethereumHead.number - BigInt(Math.floor(diff / 12)), ethereumHead.number];

    logger.info(`Requesting MerkleRoot logs from ${from} to ${to} blocks`);

    const merkleRootFromLogs = await msgQClient.findMerkleRootInRangeOfBlocks(from, to, blockNumber);

    if (merkleRootFromLogs.blockNumber === blockNumber) {
      merkleRoot = merkleRootFromLogs.merkleRoot;
      logger.info(`Received merkle root ${merkleRoot} for block ${blockNumber}`);
    } else {
      const authoritySetIdForClosestBlock = await gearClient.getAuthoritySetIdByBlockNumber(
        merkleRootFromLogs.blockNumber,
      );

      logger.info(
        `Received merkle root ${merkleRootFromLogs.merkleRoot} for block ${merkleRootFromLogs.blockNumber} with the same authority set id`,
      );

      if (authoritySetIdForClosestBlock === authoritySetId) {
        merkleRoot = merkleRootFromLogs.merkleRoot;
        blockNumber = merkleRootFromLogs.blockNumber;
        blockHash = (await gearApi.blocks.getBlockHash(Number(blockNumber))).toHex();
      }
    }
  }

  const msgHash = messageHash(msg);
  logger.info(`Fetching merkle proof for block ${blockHash} and message hash ${msgHash}`);
  const merkleProof = await gearClient.fetchMerkleProof(blockHash, msgHash);

  return msgQClient.processMessage(blockNumber, msg, merkleProof);
}
