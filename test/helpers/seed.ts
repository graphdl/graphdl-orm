import type { Payload } from 'payload'

export async function seedPersonSchema(payload: Payload) {
  // Create entity nouns
  const person = await payload.create({
    collection: 'nouns',
    data: { name: 'Person', plural: 'people', objectType: 'entity' },
  })
  const order = await payload.create({
    collection: 'nouns',
    data: { name: 'Order', plural: 'orders', objectType: 'entity' },
  })

  // Create value nouns
  const personName = await payload.create({
    collection: 'nouns',
    data: { name: 'PersonName', objectType: 'value', valueType: 'string' },
  })
  const age = await payload.create({
    collection: 'nouns',
    data: { name: 'Age', objectType: 'value', valueType: 'integer' },
  })
  const orderNumber = await payload.create({
    collection: 'nouns',
    data: { name: 'OrderNumber', objectType: 'value', valueType: 'string' },
  })

  // Set reference schemes on entity nouns (how they are uniquely identified).
  // Person is identified by PersonName, Order is identified by OrderNumber.
  await payload.update({
    collection: 'nouns',
    id: person.id,
    data: { referenceScheme: [personName.id] },
  })
  await payload.update({
    collection: 'nouns',
    id: order.id,
    data: { referenceScheme: [orderNumber.id] },
  })

  // In v3, create graph-schemas first, then create readings with graphSchema set.
  // The afterChange hook on Readings auto-creates roles.

  const personHasName = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'PersonHasPersonName',
    },
  })
  await payload.create({
    collection: 'readings',
    data: {
      text: 'Person has PersonName',
      endpointHttpVerb: 'GET',
      graphSchema: personHasName.id,
    },
  })

  const personHasAge = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'PersonHasAge',
    },
  })
  await payload.create({
    collection: 'readings',
    data: {
      text: 'Person has Age',
      endpointHttpVerb: 'GET',
      graphSchema: personHasAge.id,
    },
  })

  const personPlacesOrder = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'PersonPlacesOrder',
    },
  })
  await payload.create({
    collection: 'readings',
    data: {
      text: 'Person places Order',
      endpointHttpVerb: 'POST',
      graphSchema: personPlacesOrder.id,
    },
  })

  const orderHasNumber = await payload.create({
    collection: 'graph-schemas',
    data: {
      name: 'OrderHasOrderNumber',
    },
  })
  await payload.create({
    collection: 'readings',
    data: {
      text: 'Order has OrderNumber',
      endpointHttpVerb: 'GET',
      graphSchema: orderHasNumber.id,
    },
  })

  // Set cardinality constraints (the roleRelationship hook creates
  // UC constraints and constraint-spans)
  await payload.update({
    collection: 'graph-schemas',
    id: personHasName.id,
    data: { roleRelationship: 'many-to-one' },
  })
  await payload.update({
    collection: 'graph-schemas',
    id: personHasAge.id,
    data: { roleRelationship: 'many-to-one' },
  })
  await payload.update({
    collection: 'graph-schemas',
    id: personPlacesOrder.id,
    data: { roleRelationship: 'one-to-many' },
  })
  await payload.update({
    collection: 'graph-schemas',
    id: orderHasNumber.id,
    data: { roleRelationship: 'many-to-one' },
  })

  return {
    nouns: { person, order, personName, age, orderNumber },
    schemas: { personHasName, personHasAge, personPlacesOrder, orderHasNumber },
  }
}
