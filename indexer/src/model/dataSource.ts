import { DataSource } from 'typeorm';
import dotenv from 'dotenv';
import {
  CompletedTransfer,
  EthBridgeProgram,
  GearEthBridgeMessage,
  InitiatedTransfer,
  Pair,
  Transfer,
  VaraBridgeProgram,
} from './entities';

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
  ],
  migrations: ['db/migrations/*.js'],
});

export default AppDataSource;
