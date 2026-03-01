import type { Payload } from 'payload'

async function createFact(payload: Payload, name: string, text: string, relationship: string) {
  const schema = await payload.create({ collection: 'graph-schemas', data: { name } })
  await payload.create({ collection: 'readings', data: { text, graphSchema: schema.id } })
  await payload.update({ collection: 'graph-schemas', id: schema.id, data: { roleRelationship: relationship } })
  return schema
}

export async function seedSupportDomain(payload: Payload) {
  // Entity nouns
  const customer = await payload.create({
    collection: 'nouns',
    data: { name: 'Customer', plural: 'customers', objectType: 'entity', permissions: ['create', 'read', 'update', 'list', 'login'] },
  })
  const supportRequest = await payload.create({
    collection: 'nouns',
    data: { name: 'SupportRequest', plural: 'support-requests', objectType: 'entity', permissions: ['create', 'read', 'update', 'list'] },
  })
  const featureRequest = await payload.create({
    collection: 'nouns',
    data: { name: 'FeatureRequest', plural: 'feature-requests', objectType: 'entity', permissions: ['create', 'read', 'update', 'list'] },
  })
  const apiProduct = await payload.create({
    collection: 'nouns',
    data: { name: 'APIProduct', plural: 'api-products', objectType: 'entity', permissions: ['create', 'read', 'update', 'list'] },
  })

  // Value nouns
  const emailAddress = await payload.create({ collection: 'nouns', data: { name: 'EmailAddress', objectType: 'value', valueType: 'string', format: 'email' } })
  const subject = await payload.create({ collection: 'nouns', data: { name: 'Subject', objectType: 'value', valueType: 'string' } })
  const description = await payload.create({ collection: 'nouns', data: { name: 'Description', objectType: 'value', valueType: 'string' } })
  const channelName = await payload.create({ collection: 'nouns', data: { name: 'ChannelName', objectType: 'value', valueType: 'string', enum: 'Slack, Email' } })
  const priority = await payload.create({ collection: 'nouns', data: { name: 'Priority', objectType: 'value', valueType: 'string', enum: 'low, medium, high, urgent' } })
  const requestId = await payload.create({ collection: 'nouns', data: { name: 'RequestId', objectType: 'value', valueType: 'string', format: 'uuid' } })
  const featureRequestId = await payload.create({ collection: 'nouns', data: { name: 'FeatureRequestId', objectType: 'value', valueType: 'string', format: 'uuid' } })
  const voteCount = await payload.create({ collection: 'nouns', data: { name: 'VoteCount', objectType: 'value', valueType: 'integer', minimum: 0 } })
  const endpointSlug = await payload.create({ collection: 'nouns', data: { name: 'EndpointSlug', objectType: 'value', valueType: 'string' } })

  // Reference schemes
  await payload.update({ collection: 'nouns', id: customer.id, data: { referenceScheme: [emailAddress.id] } })
  await payload.update({ collection: 'nouns', id: supportRequest.id, data: { referenceScheme: [requestId.id] } })
  await payload.update({ collection: 'nouns', id: featureRequest.id, data: { referenceScheme: [featureRequestId.id] } })
  await payload.update({ collection: 'nouns', id: apiProduct.id, data: { referenceScheme: [endpointSlug.id] } })

  // Support request facts
  await createFact(payload, 'CustomerHasEmailAddress', 'Customer has EmailAddress', 'one-to-one')
  await createFact(payload, 'SupportRequestHasSubject', 'SupportRequest has Subject', 'many-to-one')
  await createFact(payload, 'SupportRequestHasDescription', 'SupportRequest has Description', 'many-to-one')
  await createFact(payload, 'SupportRequestArrivesViaChannelName', 'SupportRequest arrives via ChannelName', 'many-to-one')
  await createFact(payload, 'SupportRequestHasPriority', 'SupportRequest has Priority', 'many-to-one')
  await createFact(payload, 'CustomerSubmitsSupportRequest', 'Customer submits SupportRequest', 'one-to-many')
  await createFact(payload, 'SupportRequestConcernsAPIProduct', 'SupportRequest concerns APIProduct', 'many-to-many')

  // Feature request facts
  await createFact(payload, 'SupportRequestLeadsToFeatureRequest', 'SupportRequest leads to FeatureRequest', 'many-to-one')
  await createFact(payload, 'FeatureRequestHasSubject', 'FeatureRequest has Subject', 'many-to-one')
  await createFact(payload, 'FeatureRequestHasDescription', 'FeatureRequest has Description', 'many-to-one')
  await createFact(payload, 'FeatureRequestHasVoteCount', 'FeatureRequest has VoteCount', 'many-to-one')
  await createFact(payload, 'FeatureRequestConcernsAPIProduct', 'FeatureRequest concerns APIProduct', 'many-to-many')

  // API product facts
  await createFact(payload, 'APIProductHasEndpointSlug', 'APIProduct has EndpointSlug', 'one-to-one')

  return {
    nouns: { customer, supportRequest, featureRequest, apiProduct, emailAddress, subject, description, channelName, priority, requestId, featureRequestId, voteCount, endpointSlug },
  }
}

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
