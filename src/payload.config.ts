// import { payloadCloudPlugin } from '@payloadcms/payload-cloud'
// storage-adapter-import-placeholder
import { mongooseAdapter } from '@payloadcms/db-mongodb'
import { slateEditor } from '@payloadcms/richtext-slate'
import path from 'path'
import { buildConfig } from 'payload'
import sharp from 'sharp'
import { fileURLToPath } from 'url'

import ConstraintSpans from './collections/ConstraintSpans'
import Constraints from './collections/Constraints'
import EventTypes from './collections/EventTypes'
import Events from './collections/Events'
import Generator from './collections/Generator'
import GraphSchemas from './collections/GraphSchemas'
import Graphs from './collections/Graphs'
import GuardRuns from './collections/GuardRuns'
import Guards from './collections/Guards'
import JsonExamples from './collections/JsonExamples'
import Nouns from './collections/Nouns'
import Readings from './collections/Readings'
import ResourceRoles from './collections/ResourceRoles'
import Resources from './collections/Resources'
import Roles from './collections/Roles'
import StateMachineDefinitions from './collections/StateMachineDefinitions'
import StateMachines from './collections/StateMachines'
import Statuses from './collections/Statuses'
import Streams from './collections/Streams'
import Transitions from './collections/Transitions'
import { Users } from './collections/Users'
import Verbs from './collections/Verbs'

const filename = fileURLToPath(import.meta.url)
const dirname = path.dirname(filename)

export default buildConfig({
  admin: {
    user: Users.slug,
    importMap: {
      baseDir: path.resolve(dirname),
    },
  },
  collections: [
    Generator,
    JsonExamples,
    GraphSchemas,
    Readings,
    Roles,
    Constraints,
    ConstraintSpans,
    Nouns,
    Verbs,
    EventTypes,
    Streams,
    Statuses,
    StateMachineDefinitions,
    Transitions,
    Guards,
    Graphs,
    Resources,
    ResourceRoles,
    Events,
    StateMachines,
    GuardRuns,
    Users,
  ],
  editor: slateEditor({}),
  secret: process.env.PAYLOAD_SECRET || '',
  typescript: {
    outputFile: path.resolve(dirname, 'payload-types.ts'),
  },
  graphQL: {
    schemaOutputFile: path.resolve(dirname, 'generated-schema.graphql'),
  },
  db: mongooseAdapter({
    url: process.env.DATABASE_URI || 'mongodb://localhost:27017/graphdl',
    transactionOptions: process.env.PAYLOAD_DISABLE_ADMIN === 'true' ? false : {},
  }),
  sharp,
  // plugins: [
  //   payloadCloudPlugin(),
  //   // storage-adapter-placeholder
  // ],
})
