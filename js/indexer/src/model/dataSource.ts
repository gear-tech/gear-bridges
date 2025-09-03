import { DataSource } from 'typeorm';
import dotenv from 'dotenv';
import {
  CheckpointSlot,
  CompletedTransfer,
  EthBridgeProgram,
  GearEthBridgeMessage,
  InitiatedTransfer,
  MerkleRootInMessageQueue,
  Pair,
  Transfer,
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
    GearEthBridgeMessage,
    InitiatedTransfer,
    Pair,
    Transfer,
    VaraBridgeProgram,
    CheckpointSlot,
    MerkleRootInMessageQueue,
  ],
  migrations: ['db/migrations/*.js'],
});

export default AppDataSource;
