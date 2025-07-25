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
  host: process.env.DB_HOST || 'localhost',
  port: parseInt(process.env.DB_PORT || '5432'),
  username: process.env.DB_USER || 'postgres',
  password: process.env.DB_PASS || 'postgres',
  database: process.env.DB_NAME || 'gear_monitoring',
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
