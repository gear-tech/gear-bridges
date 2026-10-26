import { DataSource } from 'typeorm';
import dotenv from 'dotenv';
import {
  CompletedTransfer,
  EthBridgeProgram,
  EthereumFinalityCheckpoint,
  GearEthBridgeMessage,
  InitiatedTransfer,
  GearManagerHandoff,
  GearProgram,
  MerkleRootInMessageQueue,
  Pair,
  Transfer,
  ReceiptRelay,
  ReceiptSettlement,
  VaraBridgeProgram,
} from './entities/index.js';

dotenv.config();

const AppDataSource = new DataSource({
  type: 'postgres',
  url: process.env.DATABASE_URL,
  synchronize: false,
  migrationsRun: true,
  logging: process.env.NODE_ENV === 'development',
  entities: [
    CompletedTransfer,
    EthBridgeProgram,
    EthereumFinalityCheckpoint,
    GearEthBridgeMessage,
    GearManagerHandoff,
    GearProgram,
    InitiatedTransfer,
    Pair,
    Transfer,
    ReceiptRelay,
    ReceiptSettlement,
    VaraBridgeProgram,
    MerkleRootInMessageQueue,
  ],
  migrations: ['db/migrations/*.js'],
});

export default AppDataSource;
