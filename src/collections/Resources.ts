import { CollectionConfig } from 'payload'
import type { Graph, Resource } from '../payload-types'

const Resources: CollectionConfig = {
  slug: 'resources',
  admin: {
    group: 'Implementations',
    useAsTitle: 'title',
  },
  hooks: {
    afterChange: [
      async ({ doc, req, operation, context }) => {
        const { payload } = req
        // Recursion guard
        if ((context.internal as string[])?.includes('resources.afterChange')) return
        if (!context.internal) context.internal = []
        ;(context.internal as string[]).push('resources.afterChange')

        if (operation !== 'create' && operation !== 'update') return

        // Find state machine instances for this resource
        const stateMachines = await payload.find({
          collection: 'state-machines',
          where: { resource: { equals: doc.id } },
          depth: 2,
          req,
        })

        if (!stateMachines.docs.length) return

        for (const sm of stateMachines.docs) {
          const currentStatusId = typeof sm.stateMachineStatus === 'string' ? sm.stateMachineStatus : (sm.stateMachineStatus as any)?.id
          if (!currentStatusId) continue

          // Find transitions from the current status
          const transitions = await payload.find({
            collection: 'transitions',
            where: { from: { equals: currentStatusId } },
            depth: 3,
            req,
          })

          for (const transition of transitions.docs) {
            const verb = typeof transition.verb === 'object' ? transition.verb : null
            if (!verb) continue

            const eventType = typeof transition.eventType === 'object' ? transition.eventType : null
            if (!eventType) continue

            // Check if this verb is linked to the event type
            const linkedVerbs = (eventType as any).canBeCreatedbyVerbs || []
            const verbMatches = linkedVerbs.some((v: any) => {
              const vId = typeof v === 'string' ? v : v?.id
              return vId === (typeof verb === 'object' ? verb.id : verb)
            })

            if (!verbMatches) continue

            // Create the event
            await payload.create({
              collection: 'events',
              data: {
                type: (eventType as any).id,
                timestamp: new Date().toISOString(),
                stateMachine: (sm as any).id,
              },
              req,
            })

            // Update state machine to new status
            const toStatusId = typeof transition.to === 'string' ? transition.to : (transition.to as any)?.id
            if (toStatusId) {
              await payload.update({
                collection: 'state-machines',
                id: (sm as any).id,
                data: { stateMachineStatus: toStatusId },
                req,
              })
            }

            // Fire callback if the verb has a function with a callbackUrl
            const func = typeof (verb as any).function === 'object' ? (verb as any).function : null
            if (func?.callbackUrl) {
              try {
                await fetch(func.callbackUrl, {
                  method: func.httpMethod || 'POST',
                  headers: { 'Content-Type': 'application/json' },
                  body: JSON.stringify({
                    resourceId: doc.id,
                    event: (eventType as any).name,
                    previousStatus: currentStatusId,
                    newStatus: toStatusId,
                    resource: doc,
                  }),
                })
              } catch {
                // Log but don't block
              }
            }

            break // Only fire the first matching transition
          }
        }
      },
    ],
  },
  fields: [
    {
      name: 'title',
      type: 'text',
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ originalDoc, data, req: { payload } }) => {
            const typeId = data?.type || originalDoc?.type
            const reference: {
              relationTo: 'resources' | 'graphs'
              value: string
            }[] = data?.reference || originalDoc?.reference || []
            const resourceIds = reference
              .filter((r) => r.relationTo === 'resources')
              ?.map((r) => r.value)
              ?.join(',')
            const graphIds = reference
              .filter((r) => r.relationTo === 'graphs')
              ?.map((r) => r.value)
              ?.join(',')
            const [type, resources, graphs] = await Promise.all([
              typeId ? payload.findByID({ collection: 'nouns', id: typeId }).catch(() => null) : null,
              resourceIds
                ? payload
                    .find({ collection: 'resources', where: { id: { in: resourceIds } } })
                    .then((r) => r.docs)
                : undefined,
              graphIds
                ? payload
                    .find({ collection: 'graphs', where: { id: { in: graphIds } } })
                    .then((r) => r.docs)
                : undefined,
            ])
            const references = reference.map((r) => {
              return (
                resources?.find((res: Resource) => res.id === r.value) ||
                graphs?.find((g: Graph) => g.id === r.value)
              )
            })
            const typeName = type?.name || 'Resource'
            return `${typeName} - ${
              data?.value ||
              originalDoc?.value ||
              references
                ?.map(
                  (r) =>
                    (r as Resource).reference?.map((ref) => ref.value)?.join(', ') ||
                    (r as Resource)?.value,
                )
                ?.join(', ')
            }`
          },
        ],
      },
    },
    {
      name: 'type',
      type: 'relationship',
      relationTo: 'nouns',
      required: true,
      admin: {
        description: 'Resource is an instance of Noun.',
      },
    },
    {
      name: 'reference',
      type: 'relationship',
      hasMany: true,
      relationTo: ['resources', 'graphs'],
      admin: {
        description: 'Resource is identified by reference',
      },
    },
    {
      name: 'value',
      type: 'text',
      admin: {
        description: 'Resource has parsable value',
      },
    },
    // Bidirectional relationship child
    {
      name: 'stateMachine',
      type: 'join',
      collection: 'state-machines',
      on: 'resource',
      admin: {
        description: 'State Machine is for Resource.',
      },
    },
  ],
}

export default Resources
