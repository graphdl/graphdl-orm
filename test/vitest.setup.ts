import { MongoMemoryReplSet } from 'mongodb-memory-server'
import { beforeAll, afterAll } from 'vitest'

let mongod: MongoMemoryReplSet

beforeAll(async () => {
  mongod = await MongoMemoryReplSet.create({ replSet: { count: 1 } })
  process.env.DATABASE_URI = mongod.getUri()
  process.env.PAYLOAD_SECRET = 'test-secret-for-integration'
  process.env.PAYLOAD_DROP_DATABASE = 'true'
  process.env.PAYLOAD_DISABLE_ADMIN = 'true'
}, 120000)

afterAll(async () => {
  if (mongod) await mongod.stop()
}, 30000)
