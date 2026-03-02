import { CollectionConfig } from 'payload'
import { domainField } from './shared/domainScope'

const StateMachineDefinitions: CollectionConfig = {
  slug: 'state-machine-definitions',
  admin: {
    useAsTitle: 'title',
    group: 'State Machines',
  },
  fields: [
    domainField,
    {
      name: 'title',
      type: 'text',
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ data, req }) => {
            const { payload } = req
            const noun = await (data?.noun?.relationTo
              ? payload.findByID({
                  collection: data.noun.relationTo,
                  id: data.noun.value,
                  req,
                })
              : Promise.resolve(null))
            return `${noun?.name}`
          },
        ],
      },
    },
    // Bidirectional relationship parent
    {
      name: 'noun',
      type: 'relationship',
      relationTo: ['nouns', 'graph-schemas'],
      required: true,
      admin: { description: 'State Machine Definition is for Noun.' },
    },
    // Bidirectional relationship child
    {
      name: 'statuses',
      type: 'join',
      collection: 'statuses',
      on: 'stateMachineDefinition',
      admin: { description: 'Status is defined in State Machine Definition.' },
    },
  ],
}

export default StateMachineDefinitions
