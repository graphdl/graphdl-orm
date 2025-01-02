import {
  JSONSchema6 as JSONSchema,
  JSONSchema6Array as JSONSchemaArray,
  JSONSchema6Definition as JSONSchemaDefinition,
  JSONSchema6Object as JSONSchemaObject,
  JSONSchema6Type as JSONSchemaType,
} from 'json-schema'
import { Draft06 } from 'json-schema-library'
import _ from 'lodash'
import { CollectionBeforeChangeHook, CollectionConfig } from 'payload'
import * as gdl from '../payload-types'

const Generator: CollectionConfig = {
  slug: 'generators',
  admin: {
    group: 'Relational Mapping',
  },
  fields: [
    {
      label: 'Metadata',
      type: 'collapsible',
      admin: { initCollapsed: true },
      fields: [
        {
          label: 'Info',
          type: 'collapsible',
          admin: { initCollapsed: true },
          fields: [
            { name: 'title', type: 'text' },
            { name: 'version', type: 'text', defaultValue: '1.0' },
            {
              label: 'Contact',
              type: 'collapsible',
              fields: [
                { name: 'email', type: 'email' },
                { name: 'name', type: 'text' },
                { name: 'url', type: 'text' },
              ],
            },
            { name: 'description', type: 'textarea' },
          ],
        },
        { name: 'servers', type: 'json' },
        {
          name: 'globalPermissions',
          type: 'select',
          hasMany: true,
          options: [
            { label: 'Create', value: 'create' },
            { label: 'Read', value: 'read' },
            { label: 'Update', value: 'update' },
            { label: 'Delete', value: 'delete' },
            { label: 'List', value: 'list' },
            { label: 'Login', value: 'login' },
            { label: 'Rate Limit', value: 'rateLimit' },
          ],
          defaultValue: ['login', 'rateLimit'],
        },
        {
          label: 'Templates',
          type: 'collapsible',
          admin: {
            initCollapsed: true,
            description: 'Example objects that will be used to generate schemas to wrap data.',
          },
          fields: [
            { name: 'replacementFieldPath', type: 'text', defaultValue: 'data' },
            { name: 'globalWrapperTemplate', type: 'json' },
            { name: 'errorCodePath', type: 'text', defaultValue: 'error.code' },
            { name: 'errorMessagePath', type: 'text', defaultValue: 'error.message' },
            { name: 'errorTemplate', type: 'json' },
          ],
        },
        {
          name: 'databaseEngine',
          type: 'select',
          options: ['Payload'],
          required: true,
          defaultValue: 'Payload',
        },
      ],
    },
    { name: 'output', type: 'json' },
  ],
  hooks: {
    beforeChange: [
      (async ({ req: { payload }, data, context }) => {
        if ((context.internal as string[])?.includes('schemas.afterHook')) return
        if (!context.internal) context.internal = []
        ;(context.internal as string[])?.push('schemas.afterHook')
        // #region Retrieve data
        const schemas: Record<string, Schema> = {}

        const [graphSchemas, nouns, constraintSpans, examples, jsons] = (await Promise.all([
          payload
            .find({ collection: 'graph-schemas', pagination: false, depth: 2 })
            .then((s) => s.docs),
          payload.find({ collection: 'nouns', pagination: false }).then((n) => n.docs),
          payload
            .find({
              collection: 'constraint-spans', // get constraints
              pagination: false,
              depth: 6,
              where: { 'constraint.kind': { equals: 'UC' } }, // filter to uniqueness constraints
            })
            .then((cs) => cs.docs),
          payload
            .find({
              collection: 'graphs',
              where: { isExample: { equals: true } },
              pagination: false,
            })
            .then((n) => n.docs),
          payload
            .find({
              collection: 'json-examples',
              where: { verbatim: { equals: true } },
              pagination: false,
            })
            .then((n) => n.docs),
        ])) as [
          Omit<gdl.GraphSchema, 'updatedAt'>[],
          Omit<gdl.Noun | gdl.GraphSchema, 'updatedAt'>[],
          Omit<gdl.ConstraintSpan, 'updatedAt'>[],
          Omit<gdl.Graph, 'updatedAt'>[],
          Omit<gdl.JsonExample, 'updatedAt'>[],
        ]
        const jsonExamples = Object.fromEntries(
          jsons.map((j) => [
            nameToKey((j.noun?.value as gdl.Noun | gdl.GraphSchema)?.name || ''),
            j.jsonExample as JSONSchemaType,
          ]),
        )
        // #endregion
        // #region Find composite uniqueness schemas
        const compoundUniqueSchemas = constraintSpans
          .filter((cs) => {
            const roles = cs.roles as gdl.Role[]
            return roles?.length > 1 && roles.every((r) => r.graphSchema === roles?.[0].graphSchema)
          })
          .map((cs) => ({ gs: (cs.roles as gdl.Role[])[0].graphSchema as gdl.GraphSchema, cs }))
        const arrayTypes = compoundUniqueSchemas.filter(
          ({ gs: cs }) =>
            !graphSchemas.find((s) =>
              s.roles?.find((r) => ((r as gdl.Role).noun?.value as gdl.GraphSchema)?.id === cs.id),
            ),
        )
        const associationSchemas = compoundUniqueSchemas.filter((cs) => !arrayTypes.includes(cs))
        // #endregion
        // #region Add gerunds to noun list and add association tables
        nouns.push(...associationSchemas.map(({ gs }) => gs))
        for (const { gs: associationSchema, cs } of associationSchemas) {
          const key = (associationSchema.name || '').replace(/ /g, '')
          const jsonExample = jsonExamples[key]
          schemas['Update' + key] = {
            $id: 'Update' + key,
            title: associationSchema.name || '',
            type: 'object',
            description:
              associationSchema.description ||
              (associationSchema.readings?.[0] as gdl.Reading)?.text?.replace(/- /, ' '),
          }
          schemas['New' + key] = {
            $id: 'New' + key,
            allOf: [{ $ref: '#/components/schemas/Update' + key }],
          }
          schemas[key] = {
            $id: key,
            allOf: [{ $ref: '#/components/schemas/New' + key }],
          }
          if (jsonExample) {
            schemas['Update' + key].examples = [jsonExample]
            schemas['New' + key].examples = [jsonExample]
            schemas[key].examples = [jsonExample]
          }
          for (const role of associationSchema.roles || []) {
            const idNoun = (role as gdl.Role).noun?.value as gdl.Noun | gdl.GraphSchema
            setTableProperty({
              tables: schemas,
              subject: associationSchema,
              object: idNoun,
              nouns,
              required: cs.roles.find((r) => (r as gdl.Role).id === (role as gdl.Role).id)
                ? true
                : false,
              description: `${associationSchema.name} is uniquely identified by ${idNoun.name}`,
              property: createProperty({ object: idNoun, tables: schemas, nouns, jsonExamples }),
              jsonExamples,
            })
          }
        }
        const nounRegex = nounListToRegex(nouns)
        // #endregion
        processBinarySchemas(constraintSpans, schemas, nouns, jsonExamples, nounRegex, examples)
        // #region Map association tables with no extra properties to arrays
        processArraySchemas(arrayTypes, nouns, nounRegex, schemas, jsonExamples)
        // #endregion
        processUnarySchemas(graphSchemas, nouns, nounRegex, schemas, jsonExamples, examples)
        // #region Schema processor
        // Flatten allOf chains in order to help parsing engines that don't support them
        const componentSchemas: [string, Schema][] = Object.entries(schemas)
        for (const [key, schema] of componentSchemas) {
          while (schema.allOf) {
            const mergedRequired: string[] = [...(schema.required || [])]
            let mergedProperties = schema.properties || {}
            const mergedAllOf: JSONSchemaDefinition[] = []
            schema.allOf.forEach((s) => {
              const dependency = schemas[(s as JSONSchema).$ref?.split('/').pop() || '']
              if (dependency.required?.length)
                mergedRequired.push(
                  ...dependency.required.filter((f: string) => !mergedRequired.includes(f)),
                )
              if (Object.keys(dependency.properties || {}).length)
                mergedProperties = { ...dependency.properties, ...mergedProperties }
              if (dependency.allOf?.length) mergedAllOf.push(...dependency.allOf.map((a) => a))
              if (!schema.title && dependency.title) schema.title = dependency.title
              if (!schema.description && dependency.description)
                schema.description = dependency.description
              if (!schema.type && dependency.type) schema.type = dependency.type
              if (!schema.examples && Object.keys(dependency.examples || {}).length)
                schema.examples = dependency.examples
            })
            delete schema.allOf
            if (Object.keys(mergedProperties).length) schema.properties = mergedProperties
            if (mergedRequired.length) schema.required = mergedRequired
            if (mergedAllOf.length) schema.allOf = mergedAllOf
          }
          schemas[key] = schema
        }

        const {
          replacementFieldPath,
          globalWrapperTemplate,
          errorTemplate,
          errorCodePath,
          errorMessagePath,
          title,
          version,
          email,
          name,
          url,
          description,
          servers,
          globalPermissions,
          databaseEngine,
        } = data

        if (databaseEngine == 'Payload') {
          schemas['ListModel'] = {
            properties: {
              page: {
                type: 'number',
                description: 'Current page number',
                examples: [2],
              },
              nextPage: {
                type: 'number',
                nullable: true,
                description: "`number` of next page, `null` if it doesn't exist",
                examples: [3],
              },
              prevPage: {
                type: 'number',
                nullable: true,
                description: "`number` of previous page, `null` if it doesn't exist",
                examples: [1],
              },
              totalPages: {
                type: 'number',
                description: 'Total pages available, based upon the `limit`',
                examples: [3],
              },
              totalCount: {
                type: 'number',
                description: 'Total available records within the database',
                examples: [25],
              },
              limit: {
                type: 'number',
                description: 'Limit query parameter, defaults to `10`',
                examples: [10],
              },
              pagingCounter: {
                type: 'number',
                description: '`number` of the first record on the current page',
                examples: [11],
              },
              hasPrevPage: {
                type: 'boolean',
                description: '`true/false` if previous page exists',
                examples: [true],
              },
              hasNextPage: {
                type: 'boolean',
                description: '`true/false` if next page exists',
                examples: [true],
              },
            },
          }
        }

        const replacements: string[][] = []
        // stringify and parse later to remove undefined values
        let output = JSON.stringify({
          openapi: '3.1.0',
          info: {
            title: title || undefined,
            version: version || undefined,
            contact: {
              email: email || undefined,
              name: name || undefined,
              url: url || undefined,
            },
            description: description || undefined,
          },
          servers: servers || undefined,
          paths: Object.fromEntries(
            Object.entries(schemas)
              .filter(([key, _schema]) => schemas['Update' + key] || schemas['New' + key])
              .flatMap(([key, schema]: [string, Schema]) => {
                const baseSchema = schemas['Update' + key] || schemas['New' + key] || schema
                const idScheme: [PropertyKey, JSONSchemaDefinition][] | undefined = getIdScheme(
                  baseSchema,
                  schemas,
                )
                const subject = nouns.find((n) => nameToKey(n.name || '') === key)
                const permissions = (
                  subject?.permissions || ['create', 'read', 'update', 'list', 'login', 'rateLimit']
                ).concat(globalPermissions || [])
                const isId = idScheme?.length === 1 && idScheme[0][0] === 'id'
                const title = baseSchema.title || ''
                const retval: [string, any][] = []
                const plural =
                  (subject?.plural && subject.plural[0].toUpperCase() + subject.plural.slice(1)) ||
                  title + 's'
                const nounIsPlural = plural === title
                let postUsed = false,
                  patchUsed = false
                const {
                  unauthorizedError,
                  rateError,
                  notFoundError,
                }: {
                  unauthorizedError: object | undefined
                  rateError: object | undefined
                  notFoundError: object | undefined
                } = createErrorTemplates(
                  errorTemplate,
                  permissions,
                  errorMessagePath,
                  errorCodePath,
                  title,
                )
                // #region Add paths for CRUD operations based on permissions
                const basePath = `/${nameToKey(plural).toLowerCase()}`
                if (permissions.includes('list')) {
                  const wrapperTemplate = _.cloneDeep(globalWrapperTemplate) as Record<
                    string,
                    Schema
                  >
                  const pathParameters = []
                  const operationParameters = []
                  if (databaseEngine === 'Payload') {
                    const whereSchema: JSONSchemaDefinition = {
                      type: 'object',
                      properties: {
                        and: {
                          type: 'array',
                          items: {
                            $ref: `#/components/schemas/Where${key}`,
                          },
                        },
                        or: {
                          type: 'array',
                          items: {
                            $ref: `#/components/schemas/Where${key}`,
                          },
                        },
                      },
                    }
                    for (const [key, value] of Object.entries(baseSchema?.properties || {}) as [
                      string,
                      JSONSchema,
                    ][]) {
                      if (whereSchema.properties)
                        whereSchema.properties[key] = {
                          type: 'object',
                          properties: {
                            equals: {
                              type: value.type,
                              description: `The ${value.title || 'value'} must be exactly equal.`,
                            },
                            not_equals: {
                              type: value.type,
                              description: `The query will return all documents where the ${value.title || 'value'} is not equal.`,
                            },
                            greater_than: {
                              type: value.type,
                              description: `The ${value.title || 'value'} must be greater than.`,
                            },
                            greater_than_equal: {
                              type: value.type,
                              description: `The ${value.title || 'value'} must be greater than or equal.`,
                            },
                            less_than: {
                              type: value.type,
                              description: `The ${value.title || 'value'} must be less than.`,
                            },
                            less_than_equal: {
                              type: value.type,
                              description: `The ${value.title || 'value'} must be less than or equal.`,
                            },
                            like: {
                              type: 'string',
                              description:
                                'Case-insensitive string must be present. If string of words, all words must be present, in any order.',
                            },
                            contains: {
                              type: 'string',
                              description: `Must contain the ${value.title || 'value'} entered, case-insensitive.`,
                            },
                            in: {
                              type: 'string',
                              description: `The ${value.title || 'value'} must be found within the provided comma-delimited list of values.`,
                            },
                            not_in: {
                              type: 'string',
                              description: `The ${value.title || 'value'} must NOT be within the provided comma-delimited list of values.`,
                            },
                            all: {
                              type: 'string',
                              description: `The ${value.title || 'value'} must contain all values provided in the comma-delimited list.`,
                            },
                            exists: {
                              type: 'boolean',
                              description: `Only return documents where the ${value.title || 'value'} either exists (true) or does not exist (false).`,
                            },
                            // near: {
                            //   type: 'string',
                            //   description:
                            //     'For distance related to a point field comma separated as <longitude>, <latitude>, <maxDistance in meters (nullable)>, <minDistance in meters (nullable)>.',
                            // },
                          },
                        }
                      const whereProperty = whereSchema.properties?.[key] as JSONSchema
                      if (
                        whereProperty?.properties &&
                        value.type !== 'number' &&
                        value.format !== 'date-time' &&
                        value.format !== 'date'
                      ) {
                        delete whereProperty.properties.greater_than
                        delete whereProperty.properties.greater_than_equal
                        delete whereProperty.properties.less_than
                        delete whereProperty.properties.less_than_equal
                      }
                    }
                    pathParameters.push({
                      schema: { type: 'integer' },
                      name: 'depth',
                      in: 'query',
                      required: false,
                      description:
                        'The number of levels of related objects to include in the response',
                    })
                    operationParameters.push(
                      {
                        schema: {
                          type: 'string',
                        },
                        in: 'query',
                        name: 'sort',
                        description:
                          'Pass the name of a top-level field to sort by that field in ascending order. Prefix the name of the field with a minus symbol ("-") to sort in descending order.',
                      },
                      {
                        schema: {
                          type: 'number',
                        },
                        in: 'query',
                        name: 'limit',
                        description: 'Limit number of results, default 10',
                      },
                      {
                        schema: {
                          $ref: `#/components/schemas/Where${key}`,
                        },
                        in: 'query',
                        name: 'where',
                        description:
                          'Search for results fitting criteria, uses qs library for query string parsing',
                      },
                    )
                    schemas['Where' + key] = whereSchema
                  }
                  const filledSchema = fillSchemaTemplate({
                    schema: {
                      type: 'array',
                      items: { $ref: `#/components/schemas/${schema.$id}` },
                    },
                    wrapperTemplate,
                    replacementFieldPath,
                  })
                  retval.push([
                    basePath,
                    {
                      parameters: pathParameters?.length ? pathParameters : undefined,
                      get: {
                        summary: `Get ${plural}`,
                        operationId: `get-${nameToKey(plural).toLowerCase()}-list`,
                        responses: {
                          '200': {
                            description: `${plural} Found`,
                            content: {
                              'application/json': {
                                schema:
                                  databaseEngine === 'Payload'
                                    ? {
                                        allOf: [
                                          filledSchema,
                                          { $ref: '#/components/schemas/ListModel' },
                                        ],
                                      }
                                    : filledSchema,
                              },
                            },
                          },
                          '401':
                            permissions.includes('login') && unauthorizedError
                              ? {
                                  description: 'Unauthorized',
                                  content: {
                                    'application/json': {
                                      schema: {
                                        $ref: '#/components/schemas/ErrorModel',
                                        examples: [unauthorizedError],
                                      },
                                    },
                                  },
                                }
                              : undefined,
                          '429':
                            permissions.includes('rateLimit') && rateError
                              ? {
                                  description: 'Too Many Requests',
                                  content: {
                                    'application/json': {
                                      schema: {
                                        $ref: '#/components/schemas/ErrorModel',
                                        examples: [rateError],
                                      },
                                    },
                                  },
                                }
                              : undefined,
                        },
                        parameters: operationParameters?.length ? operationParameters : undefined,
                      },
                    },
                  ])
                }
                if (permissions.includes('create')) {
                  const createSchema: JSONSchema | undefined = fillSchemaTemplate({
                    schema: { $ref: `#/components/schemas/${schema.$id}` },
                    wrapperTemplate: globalWrapperTemplate as Record<string, Schema>,
                    replacementFieldPath,
                  })
                  postUsed = true
                  let createError: object | undefined = undefined
                  if (errorTemplate) {
                    createError = _.cloneDeep(errorTemplate) as object
                    _.set(
                      createError,
                      errorMessagePath || 'error.message',
                      'Missing Required Information',
                    )
                    _.set(createError, errorCodePath || 'error.code', 400)
                  }

                  const create = {
                    summary: `${isId ? 'Create' : 'Add'} ${nounIsPlural ? '' : 'a '}new ${title}`,
                    operationId: `post-${key.toLowerCase()}`,
                    responses: {
                      '200': {
                        description: `${title} ${isId ? 'Created' : 'Added'}`,
                        content: {
                          'application/json': { schema: createSchema },
                        },
                      },
                      '400': createError
                        ? {
                            description: 'Missing Required Information',
                            content: createError
                              ? {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [createError],
                                    },
                                  },
                                }
                              : undefined,
                          }
                        : undefined,
                      '401':
                        permissions.includes('login') && unauthorizedError
                          ? {
                              description: 'Unauthorized',
                              content: {
                                'application/json': {
                                  schema: {
                                    $ref: '#/components/schemas/ErrorModel',
                                    examples: [unauthorizedError],
                                  },
                                },
                              },
                            }
                          : undefined,
                      '429':
                        permissions.includes('rateLimit') && rateError
                          ? {
                              description: 'Too Many Requests',
                              content: {
                                'application/json': {
                                  schema: {
                                    $ref: '#/components/schemas/ErrorModel',
                                    examples: [rateError],
                                  },
                                },
                              },
                            }
                          : undefined,
                    },
                    requestBody: {
                      content: {
                        'application/json': {
                          schema: {
                            $ref: `#/components/schemas/${(schemas['New' + key] || schema).$id}`,
                          },
                        },
                      },
                    },
                  }

                  if (retval[retval.length - 1]?.[0] === basePath)
                    retval[retval.length - 1][1].post = create
                  else {
                    const parameters = []
                    if (databaseEngine === 'Payload') {
                      parameters.push({
                        schema: { type: 'integer' },
                        name: 'depth',
                        in: 'query',
                        required: false,
                        description:
                          'The number of levels of related objects to include in the response',
                      })
                    }
                    retval.push([
                      basePath,
                      { parameters: parameters?.length ? parameters : undefined, post: create },
                    ])
                  }
                }
                if (
                  permissions.includes('read') ||
                  permissions.includes('update') ||
                  permissions.includes('delete')
                ) {
                  const parameters =
                    (idScheme?.length &&
                      idScheme.map((id) => ({
                        schema: { type: (id[1] as JSONSchema).type },
                        name: id[0],
                        in: 'path',
                        required: true,
                        description: (id[1] as JSONSchema).description,
                      }))) ||
                    []
                  if (databaseEngine === 'Payload') {
                    parameters.push({
                      schema: { type: 'integer' },
                      name: 'depth',
                      in: 'query',
                      required: false,
                      description:
                        'The number of levels of related objects to include in the response',
                    })
                  }
                  retval.push([
                    `${basePath}/${idScheme?.map((i) => `{${i[0].toString()}}`)?.join('/')}`,
                    {
                      parameters: parameters?.length ? parameters : undefined,
                    },
                  ])
                  if (permissions.includes('read')) {
                    const readSchema: JSONSchema | undefined = fillSchemaTemplate({
                      schema: { $ref: `#/components/schemas/${schema.$id}` },
                      wrapperTemplate: globalWrapperTemplate as Record<string, Schema>,
                      replacementFieldPath,
                    })
                    retval[retval.length - 1][1].get = {
                      responses: {
                        '200': {
                          description: `${title} Found`,
                          content: {
                            'application/json': {
                              schema: readSchema,
                            },
                          },
                        },
                        '404': notFoundError
                          ? {
                              description: `${title} Not Found`,
                              content: {
                                'application/json': {
                                  schema: {
                                    $ref: '#/components/schemas/ErrorModel',
                                    examples: [notFoundError],
                                  },
                                },
                              },
                            }
                          : undefined,
                        '401':
                          permissions.includes('login') && unauthorizedError
                            ? {
                                description: 'Unauthorized',
                                content: {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [unauthorizedError],
                                    },
                                  },
                                },
                              }
                            : undefined,
                        '429':
                          permissions.includes('rateLimit') && rateError
                            ? {
                                description: 'Too Many Requests',
                                content: {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [rateError],
                                    },
                                  },
                                },
                              }
                            : undefined,
                      },
                      summary: `Retrieve ${nounIsPlural ? '' : `a${['A', 'E', 'I', 'O', 'U'].includes(title[0].toUpperCase()) ? 'n ' : ' '}`}${title}`,
                      operationId: `get-${key.toLowerCase()}`,
                    }
                  }
                  if (permissions.includes('update')) {
                    const updateSchema: JSONSchema | undefined = fillSchemaTemplate({
                      schema: { $ref: `#/components/schemas/${schema.$id}` },
                      wrapperTemplate: globalWrapperTemplate as Record<string, Schema>,
                      replacementFieldPath,
                    })
                    patchUsed = true
                    retval[retval.length - 1][1].patch = {
                      responses: {
                        '200': {
                          description: `${title} Updated`,
                          content: {
                            'application/json': {
                              schema: updateSchema,
                            },
                          },
                        },
                        '404': notFoundError
                          ? {
                              description: `${title} Not Found`,
                              content: {
                                'application/json': {
                                  schema: {
                                    $ref: '#/components/schemas/ErrorModel',
                                    examples: [notFoundError],
                                  },
                                },
                              },
                            }
                          : undefined,
                        '401':
                          permissions.includes('login') && unauthorizedError
                            ? {
                                description: 'Unauthorized',
                                content: {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [unauthorizedError],
                                    },
                                  },
                                },
                              }
                            : undefined,
                        '429':
                          permissions.includes('rateLimit') && rateError
                            ? {
                                description: 'Too Many Requests',
                                content: {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [rateError],
                                    },
                                  },
                                },
                              }
                            : undefined,
                      },
                      requestBody: {
                        content: {
                          'application/json': {
                            schema: {
                              $ref: `#/components/schemas/${(schemas['Update' + key] || schemas['New' + key] || schema).$id}`,
                            },
                          },
                        },
                      },
                      summary: `Update ${nounIsPlural ? '' : 'an '}existing ${title}`,
                      operationId: `patch-${key.toLowerCase()}`,
                    }
                  }
                  if (permissions.includes('delete')) {
                    const deleteSchema: JSONSchema | undefined = fillSchemaTemplate({
                      wrapperTemplate: globalWrapperTemplate as Record<string, Schema>,
                    })
                    retval[retval.length - 1][1].delete = {
                      responses: {
                        '200': {
                          description: `${title} Deleted`,
                          content: deleteSchema
                            ? {
                                'application/json': {
                                  schema: deleteSchema,
                                },
                              }
                            : undefined,
                        },
                        '404': notFoundError
                          ? {
                              description: `${title} Not Found`,
                              content: {
                                'application/json': {
                                  schema: {
                                    $ref: '#/components/schemas/ErrorModel',
                                    examples: [notFoundError],
                                  },
                                },
                              },
                            }
                          : undefined,
                        '401':
                          permissions.includes('login') && unauthorizedError
                            ? {
                                description: 'Unauthorized',
                                content: {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [unauthorizedError],
                                    },
                                  },
                                },
                              }
                            : undefined,
                        '429':
                          permissions.includes('rateLimit') && rateError
                            ? {
                                description: 'Too Many Requests',
                                content: {
                                  'application/json': {
                                    schema: {
                                      $ref: '#/components/schemas/ErrorModel',
                                      examples: [rateError],
                                    },
                                  },
                                },
                              }
                            : undefined,
                      },
                      summary: `Delete ${nounIsPlural ? '' : 'an '}existing ${title}`,
                      operationId: `delete-${key.toLowerCase()}`,
                    }
                  }
                }
                // #endregion
                removeDuplicateSchemas(patchUsed, schemas, key, replacements, postUsed)
                return retval
              }),
          ),
          components: {
            schemas: errorTemplate
              ? { ...schemas, ErrorModel: new Draft06().createSchemaOf(errorTemplate) }
              : schemas,
          },
        })
        // #endregion
        // #region Post-processing steps

        // We don't know if a parent new/update schema had been merged until it happens, so use the replacements list to fix references
        for (const [key, replacement] of replacements) {
          output = output.replace(new RegExp(key, 'g'), replacement)
        }

        data.output = JSON.parse(output)

        // #endregion
      }) as CollectionBeforeChangeHook<gdl.Generator>,
    ],
  },
}

export default Generator

// #region Fact type processors
function processArraySchemas(
  arrayTypes: { gs: gdl.GraphSchema; cs: Omit<gdl.ConstraintSpan, 'updatedAt'> }[],
  nouns: Omit<gdl.GraphSchema | gdl.Noun, 'updatedAt'>[],
  nounRegex: RegExp,
  schemas: Record<string, Schema>,
  jsonExamples: { [k: string]: JSONSchemaType },
) {
  for (const { gs: schema } of arrayTypes) {
    const reading = schema.readings?.[0] as gdl.Reading
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex })
    const subject = (schema.roles?.[0] as gdl.Role).noun?.value as gdl.Noun | gdl.GraphSchema
    const object = (schema.roles?.[1] as gdl.Role).noun?.value as gdl.Noun | gdl.GraphSchema
    const plural = object?.plural

    const { objectBegin, objectEnd } = findPredicateObject({ predicate, subject, object, plural })
    const objectReading = predicate
      .slice(objectBegin, objectEnd)
      .map((n) => n[0].toUpperCase() + n.slice(1).replace(/-$/, ''))
    predicate.splice(objectBegin, objectReading.length, ...objectReading)
    let propertyName = schema.name || extractPropertyName(objectReading) + (plural ? '' : 's')
    propertyName = propertyName[0].toLowerCase() + propertyName.slice(1)

    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })
    const key = nameToKey('Update' + (subject.name || ''))
    const properties = schemas[key].properties ?? {}

    const property: JSONSchemaDefinition = {
      type: 'array',
      items: createProperty({ object, nouns, tables: schemas, jsonExamples }),
    }
    property.description = predicate.join(' ')
    properties[propertyName] = property
    schemas[key].properties = properties
  }
}

function processBinarySchemas(
  constraintSpans: Omit<gdl.ConstraintSpan, 'updatedAt'>[],
  schemas: Record<string, Schema>,
  nouns: Omit<gdl.GraphSchema | gdl.Noun, 'updatedAt'>[],
  jsonExamples: { [k: string]: JSONSchemaType },
  nounRegex: RegExp,
  examples: Omit<gdl.Graph, 'updatedAt'>[],
) {
  for (const propertySchema of constraintSpans
    .filter((cs) => (cs.roles as gdl.Role[])?.length === 1)
    .map((cs) => (cs.roles as gdl.Role[])[0].graphSchema as gdl.GraphSchema)) {
    const subjectRole = propertySchema.roles?.find((r) =>
      (r as gdl.Role).constraints?.some(
        (c) => ((c.value as gdl.ConstraintSpan)?.constraint as gdl.Constraint).kind === 'UC',
      ),
    ) as gdl.Role

    const subject = subjectRole.noun?.value as gdl.Noun | gdl.GraphSchema
    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })

    const objectRole = propertySchema.roles?.find((r) => (r as gdl.Role).id !== subjectRole.id)
    const object = (objectRole as gdl.Role)?.noun?.value as gdl.Noun | gdl.GraphSchema
    const reading = propertySchema.readings?.[0] as gdl.Reading
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex })
    const { objectBegin, objectEnd } = findPredicateObject({ predicate, subject, object })

    const objectReading = predicate
      .slice(objectBegin, objectEnd)
      .map((n) => n[0].toUpperCase() + n.slice(1).replace(/-$/, ''))
    predicate.splice(objectBegin, objectReading.length, ...objectReading)

    const required = subjectRole.constraints
      ?.filter(
        (c) => ((c.value as gdl.ConstraintSpan)?.constraint as gdl.Constraint)?.kind === 'MR',
      )
      .map(
        (c) =>
          propertySchema.roles?.find(
            (r) => (r as gdl.Role).id !== ((c.value as gdl.ConstraintSpan).roles[0] as gdl.Role).id,
          ) as gdl.Role,
      )

    let example = undefined
    const exampleProperty = examples.find(
      (g) => (g.type as gdl.GraphSchema)?.id === propertySchema.id,
    )
    if (exampleProperty)
      example = (
        (exampleProperty?.resourceRoles as gdl.ResourceRole[])?.find(
          (role) => (objectRole as gdl.Role).id === (role.role as gdl.Role)?.id,
        )?.resource?.value as gdl.Resource
      )?.value

    setTableProperty({
      tables: schemas,
      subject,
      object: object as gdl.Noun,
      nouns,
      propertyName: extractPropertyName(objectReading),
      description: predicate.join(' '),
      required: (required?.length || 0) > 0,
      property: createProperty({
        object: object as gdl.Noun,
        nouns,
        tables: schemas,
        jsonExamples,
      }),
      example,
      jsonExamples,
    })
  }
}

function processUnarySchemas(
  graphSchemas: Omit<gdl.GraphSchema, 'updatedAt'>[],
  nouns: Omit<gdl.Noun | gdl.GraphSchema, 'updatedAt'>[],
  nounRegex: RegExp,
  schemas: Record<string, Schema>,
  jsonExamples: { [k: string]: JSONSchemaType },
  examples: Omit<gdl.Graph, 'updatedAt'>[],
) {
  for (const unarySchema of graphSchemas.filter((s) => s.roles?.length === 1)) {
    const unaryRole = unarySchema.roles?.[0] as gdl.Role
    const subject = unaryRole?.noun?.value as gdl.Noun | gdl.GraphSchema
    const reading = (unarySchema.readings as gdl.Reading[])?.[0]
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex })
    const { objectBegin } = findPredicateObject({ predicate, subject })
    const objectReading = predicate.slice(objectBegin)

    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })

    let example = undefined
    const exampleProperty = examples.find((g) => (g.type as gdl.GraphSchema)?.id === unarySchema.id)
    if (exampleProperty)
      example = (
        (exampleProperty?.resourceRoles as gdl.ResourceRole[])?.find(
          (role) => unaryRole.id === (role.role as gdl.Role)?.id,
        )?.resource?.value as gdl.Resource
      )?.value

    const required = unaryRole.constraints
      ?.filter(
        (c) => ((c.value as gdl.ConstraintSpan)?.constraint as gdl.Constraint)?.kind === 'MR',
      )
      .map(
        (c) =>
          unarySchema.roles?.find(
            (r) => (r as gdl.Role).id !== ((c.value as gdl.ConstraintSpan).roles[0] as gdl.Role).id,
          ) as gdl.Role,
      )

    setTableProperty({
      tables: schemas,
      subject: subject,
      object: subject as gdl.Noun,
      nouns,
      propertyName: extractPropertyName(objectReading),
      description: predicate.join(' '),
      required: (required?.length || 0) > 0,
      property: { type: 'boolean' },
      example,
      jsonExamples,
    })
  }
}
// #endregion
// #region Helper functions
function removeDuplicateSchemas(
  patchUsed: boolean,
  schemas: Record<string, Schema>,
  key: string,
  replacements: string[][],
  postUsed: boolean,
) {
  if (
    !patchUsed ||
    (((!schemas['Update' + key].required && !schemas['New' + key].required) ||
      schemas['Update' + key].required?.length === schemas['New' + key].required?.length) &&
      ((!schemas['Update' + key].properties && !schemas['New' + key].properties) ||
        Object.keys(schemas['Update' + key].properties || {}).length ===
          Object.keys(schemas['New' + key].properties || {}).length))
  ) {
    patchUsed = false
    delete schemas['New' + key].allOf
    const required = schemas['New' + key].required
    schemas['New' + key] = {
      ...schemas['Update' + key],
      ...schemas['New' + key],
      required: required?.length ? required : undefined,
    }
    delete schemas['Update' + key]
    replacements.push([`Update${key}`, postUsed ? `New${key}` : key])
  }
  if (
    !postUsed ||
    (((!schemas['New' + key].required && !schemas[key].required) ||
      schemas['New' + key].required?.length === schemas[key].required?.length) &&
      ((!schemas['New' + key].properties && !schemas[key].properties) ||
        Object.keys(schemas['New' + key].properties || {}).length ===
          Object.keys(schemas[key].properties || {}).length))
  ) {
    postUsed = false
    const allOf = schemas[key].allOf
    if (!schemas['New' + key]?.allOf) delete schemas[key].allOf
    else if (
      allOf?.[1] &&
      (schemas['New' + key].allOf?.[0] as JSONSchema).$ref?.replace(/Update|New/, '') ===
        (allOf[1] as Schema).$ref
    )
      allOf.splice(0, 1)
    else if (allOf) allOf[0] = schemas['New' + key].allOf?.[0] as Schema

    const required = schemas[key].required
    schemas[key] = {
      ...schemas['New' + key],
      ...schemas[key],
      required: required?.length ? required : undefined,
    }
    delete schemas['New' + key]
    replacements.push([`New${key}`, key])
  }
}

function createErrorTemplates(
  errorTemplate:
    | string
    | number
    | boolean
    | unknown[]
    | { [k: string]: unknown }
    | null
    | undefined,
  permissions: (
    | 'create'
    | 'read'
    | 'update'
    | 'delete'
    | 'list'
    | 'login'
    | 'rateLimit'
    | 'versioned'
  )[],
  errorMessagePath: string | null | undefined,
  errorCodePath: string | null | undefined,
  title: string,
) {
  let unauthorizedError: object | undefined = undefined,
    rateError: object | undefined = undefined,
    notFoundError: object | undefined = undefined
  if (errorTemplate) {
    if (permissions.includes('login')) {
      unauthorizedError = _.cloneDeep(errorTemplate) as object
      _.set(unauthorizedError, errorMessagePath || 'error.message', 'Unauthorized')
      _.set(unauthorizedError, errorCodePath || 'error.code', 401)
    }

    if (permissions.includes('rateLimit')) {
      rateError = _.cloneDeep(errorTemplate) as object
      _.set(rateError, errorMessagePath || 'error.message', 'Too Many Requests')
      _.set(rateError, errorCodePath || 'error.code', 429)
    }
    notFoundError = _.cloneDeep(errorTemplate) as object
    _.set(notFoundError, errorMessagePath || 'error.message', `${title} Not Found`)
    _.set(notFoundError, errorCodePath || 'error.code', 404)
  }
  return { unauthorizedError, rateError, notFoundError }
}

function getIdScheme(baseSchema: Schema, schemas: Record<string, Schema>) {
  let idSchema = baseSchema
  let idScheme: [PropertyKey, JSONSchemaDefinition][] | undefined = idSchema?.properties
    ? Object.entries(idSchema.properties)?.filter((p) =>
        (p[1] as JSONSchema)?.description?.includes('is uniquely identified by'),
      )
    : undefined
  while (!idScheme?.length && idSchema?.allOf) {
    const key = (idSchema.allOf[0] as JSONSchema)?.$ref?.split('/')?.pop() as string
    const bareKey = key.replace(/^New|^Update/, '')
    idSchema = schemas['Update' + bareKey] || schemas['New' + bareKey] || schemas[bareKey]
    idScheme =
      idSchema?.properties &&
      Object.entries(idSchema.properties)?.filter((p) =>
        (p[1] as JSONSchema)?.description?.includes('is uniquely identified by'),
      )
  }

  if (idScheme?.length)
    for (let i = 0; i < idScheme.length; i++) {
      let value = idScheme[i][1] as Schema
      while (value.oneOf) {
        const description = value.description
        value = value.oneOf[0] as Schema
        if (value.properties) {
          idScheme.splice(
            i,
            1,
            ...(Object.entries(value.properties).map((p: [PropertyKey, JSONSchemaDefinition]) => [
              p[0],
              { description, ...(p[1] as JSONSchema) },
            ]) as [PropertyKey, JSONSchemaDefinition][]),
          )
          value = idScheme[i][1] as Schema
        } else idScheme[i][1] = { description, ...value }
      }
    }
  return idScheme
}

function fillSchemaTemplate({
  schema,
  example,
  wrapperTemplate,
  replacementFieldPath,
}: {
  schema?: JSONSchema
  example?: unknown
  wrapperTemplate?: Record<string, Schema>
  replacementFieldPath?: string | null
}) {
  const jsonSchema = new Draft06()
  if (wrapperTemplate && Object.keys(wrapperTemplate).length) {
    const wrapperSchema = jsonSchema.createSchemaOf(wrapperTemplate)
    if (replacementFieldPath && _.get(wrapperSchema.properties, replacementFieldPath))
      if (schema) _.set(wrapperSchema.properties, replacementFieldPath, schema)
      else _.unset(wrapperSchema.properties, replacementFieldPath)
    if (example) {
      wrapperTemplate[replacementFieldPath || ''] = example
      wrapperSchema.examples = [wrapperTemplate]
    }
    schema = wrapperSchema
  }
  return schema
}

function createProperty({
  description,
  object,
  nouns,
  tables,
  jsonExamples,
}: {
  description?: string
  object: Omit<gdl.Noun | gdl.GraphSchema, 'updatedAt'>
  nouns: Omit<gdl.Noun | gdl.GraphSchema, 'updatedAt'>[]
  tables: { [id: string]: JSONSchema }
  jsonExamples: {
    [id: string]: JSONSchemaType
  }
}) {
  object = nouns.find((n) => n.id === object.id) || object
  const property: Schema = {}
  let { referenceScheme, superType, valueType } = object as gdl.Noun
  if (!referenceScheme) {
    referenceScheme =
      (object as gdl.GraphSchema).roles?.map((r) => {
        return (r as gdl.Role).noun?.value as gdl.Noun
      }) || []
  }
  while (!referenceScheme?.length && !valueType && superType) {
    if (typeof superType === 'string') superType = nouns.find((n) => n.id === superType) as gdl.Noun
    referenceScheme = superType?.referenceScheme
    valueType = superType?.valueType
    superType = superType?.superType
  }
  if (valueType) {
    property.type = valueType
    const noun = object as gdl.Noun
    if (noun.format) property.format = noun.format?.toString()
    if (noun.pattern) property.pattern = noun.pattern?.toString()
    if (noun.enum)
      property.enum = noun.enum.split(',').map((e) => {
        const val = e.trim()
        if (val === 'null') {
          property.nullable = true
          return null
        }
        return val
      })
    if (typeof noun.minLength === 'number') property.minLength = noun.minLength
    if (typeof noun.maxLength === 'number') property.maxLength = noun.maxLength
    if (typeof noun.minimum === 'number') property.minimum = noun.minimum
    if (typeof noun.exclusiveMinimum === 'number') property.exclusiveMinimum = noun.exclusiveMinimum
    if (typeof noun.exclusiveMaximum === 'number') property.exclusiveMaximum = noun.exclusiveMaximum
    if (typeof noun.maximum === 'number') property.maximum = noun.maximum
    if (typeof noun.multipleOf === 'number') property.multipleOf = noun.multipleOf
    if (description) property.description = description
  } else {
    if (typeof referenceScheme === 'string')
      referenceScheme = [nouns.find((n) => n.id === referenceScheme?.toString()) as gdl.Noun]
    const required: string[] = []
    const propertyKey = nameToKey(object.name || '')
    property.oneOf = [
      (referenceScheme?.length || 0) > 1
        ? {
            type: 'object',
            properties: Object.fromEntries(
              referenceScheme?.map((role) => {
                if (typeof role === 'string') role = nouns.find((n) => n.id === role) as gdl.Noun
                const propertyName = transformPropertyName(role.name || '')
                required.push(propertyName)
                return [
                  propertyName,
                  createProperty({ object: role, tables, nouns, description, jsonExamples }),
                ]
              }) || [],
            ),
            required,
          }
        : referenceScheme
          ? createProperty({
              object:
                typeof referenceScheme[0] === 'string'
                  ? (nouns.find((n) => n.id === referenceScheme?.[0]) as gdl.Noun)
                  : referenceScheme[0],
              tables,
              nouns,
              description,
              jsonExamples,
            })
          : {},
      { $ref: '#/components/schemas/' + propertyKey },
    ]
    ensureTableExists({ tables, subject: object, nouns, jsonExamples })
  }
  return property
}

function nameToKey(name: string) {
  return name.replace(/[ \-]/g, '').replace(/&/g, 'And')
}

function ensureTableExists({
  tables,
  subject,
  nouns,
  jsonExamples,
}: {
  tables: { [id: string]: JSONSchema }
  subject: Omit<gdl.GraphSchema | gdl.Noun, 'updatedAt'>
  nouns: Omit<gdl.Noun, 'updatedAt'>[]
  jsonExamples: {
    [id: string]: JSONSchemaType
  }
}) {
  const title = subject.name || ''
  const key = nameToKey(title)
  if (tables[key]) return
  tables['Update' + key] = {
    $id: 'Update' + key,
    title: subject.name || '',
  }
  tables['New' + key] = {
    $id: 'New' + key,
    allOf: [{ $ref: '#/components/schemas/Update' + key }],
  }
  tables[key] = {
    $id: key,
    allOf: [{ $ref: '#/components/schemas/New' + key }],
  }
  if (subject.description) tables['Update' + key].description = subject.description
  const json = jsonExamples[key]
  if (json) {
    tables['Update' + key].examples = [json]
    tables['New' + key].examples = [json]
    tables[key].examples = [json]
  }

  // Unpack black-box columns
  if ((subject as gdl.Noun).referenceScheme) {
    let { referenceScheme } = subject as gdl.Noun
    if (!(referenceScheme instanceof Array))
      referenceScheme = [nouns.find((n) => n.id === referenceScheme?.toString()) as gdl.Noun]
    for (let idRole of referenceScheme || []) {
      if (typeof idRole === 'string') idRole = nouns.find((n) => n.id === idRole) as gdl.Noun
      const property = createProperty({ object: idRole, nouns, tables, jsonExamples })
      setTableProperty({
        tables,
        subject,
        object: idRole as gdl.Noun,
        nouns,
        required: true,
        property,
        description: `${title} is uniquely identified by ${idRole.name}`,
        jsonExamples,
      })
    }
  }

  let superType: Omit<gdl.GraphSchema | gdl.Noun, 'updatedAt'> | string | undefined | null = (
    subject as gdl.Noun
  ).superType
  if (typeof superType === 'string') superType = nouns?.find((n) => n.id === superType)
  if ((superType as gdl.Noun)?.name) {
    superType = (superType as gdl.Noun) || nouns?.find((n) => n.id === (superType as gdl.Noun).id)
    const superTypeKey = nameToKey((superType as gdl.Noun).name || '')
    tables['Update' + key].allOf = [{ $ref: '#/components/schemas/Update' + superTypeKey }]
    tables['New' + key].allOf?.push({ $ref: '#/components/schemas/New' + superTypeKey })
    tables[key].allOf?.push({ $ref: '#/components/schemas/' + superTypeKey })
    ensureTableExists({ tables, subject: superType, nouns, jsonExamples })
  } else tables['Update' + key].type = 'object'
}

export function findPredicateObject({
  predicate,
  subject,
  object,
  plural,
}: {
  predicate: string[]
  subject: gdl.GraphSchema | gdl.Noun
  object?: gdl.GraphSchema | gdl.Noun
  plural?: string | null | undefined
}) {
  let subjectIndex = predicate.indexOf(subject.name || '')
  if (subjectIndex === -1 && subject.name)
    subjectIndex = predicate.indexOf(subject.name + '-' || '')
  if (subjectIndex === -1)
    throw new Error(`Subject "${subject.name}" not found in predicate "${predicate.join(' ')}"`)

  let objectIndex = !object ? -1 : predicate.indexOf(object.name || '')
  if (object && objectIndex === -1 && object.name)
    objectIndex = predicate.indexOf(object.name + '-' || '')
  if (object && objectIndex === -1)
    throw new Error(`Object "${object.name}" not found in predicate "${predicate.join(' ')}"`)

  if (plural) predicate[objectIndex] = plural[0].toUpperCase() + plural.slice(1)
  let objectBegin, objectEnd
  if (objectIndex === -1) {
    objectBegin = subjectIndex + 1
    objectEnd = predicate.length
  } else if (subjectIndex < objectIndex) {
    objectBegin = subjectIndex + 1
    objectEnd = predicate[objectIndex].endsWith('-') ? predicate.length : objectIndex + 1
  } else {
    objectBegin = 0
    objectEnd = objectIndex + 1
  }
  while (objectIndex > -1 && !predicate[objectBegin].endsWith('-') && objectBegin !== objectIndex)
    objectBegin++
  return { objectBegin, objectEnd }
}

export function nounListToRegex(nouns?: Omit<gdl.GraphSchema | gdl.Noun, 'updatedAt'>[]) {
  return nouns
    ? new RegExp(
        '(' +
          nouns
            .filter((n) => n.name)
            .map((n) => '\\b' + n.name + '\\b-?')
            .sort((a, b) => b.length - a.length)
            .join('|') +
          ')',
      )
    : new RegExp('')
}

export function toPredicate({
  reading,
  nouns,
  nounRegex,
}: {
  reading: string
  nouns: Omit<gdl.Noun | gdl.GraphSchema, 'updatedAt'>[]
  nounRegex?: RegExp
}) {
  // tokenize by noun names and then by space
  return reading
    .split(nounRegex || nounListToRegex(nouns))
    .flatMap((token) =>
      nouns.find((n) => n.name === token.replace(/-$/, ''))
        ? token
        : token
            .trim()
            .split(' ')
            .map((word) => word.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())),
    )
    .filter((word) => word)
}

function setTableProperty({
  tables,
  nouns,
  subject,
  object,
  propertyName,
  description,
  required,
  property,
  example,
  jsonExamples,
}: {
  tables: { [id: string]: JSONSchema }
  nouns: Omit<gdl.Noun, 'updatedAt'>[]
  subject: Omit<gdl.GraphSchema | gdl.Noun, 'updatedAt'>
  object: gdl.Noun
  propertyName?: string
  description?: string
  required?: boolean
  property?: JSONSchema
  example?: JSONSchemaType
  default?: JSONSchemaType
  jsonExamples: {
    [id: string]: JSONSchemaType
  }
}) {
  if (!property) property = createProperty({ object, tables, nouns, jsonExamples })
  if (description) property.description = description

  propertyName ||= transformPropertyName(object.name || '')
  // If the property name starts with the object name, remove the object name from the property name
  const compareName = subject.name?.replace(/ /g, '')?.toUpperCase() || ''
  if (
    subject.name &&
    propertyName.toUpperCase().startsWith(compareName) &&
    propertyName.length > compareName.length &&
    propertyName[compareName.length] === propertyName[compareName.length].toUpperCase()
  ) {
    propertyName = transformPropertyName(propertyName.slice(compareName.length))
  }
  const key = nameToKey('Update' + (subject.name || ''))
  const properties = tables[key].properties ?? {}
  properties[propertyName] = property
  tables[key].properties = properties

  if (required) {
    const key = nameToKey((propertyName === 'id' ? '' : 'New') + (subject.name || ''))
    if (!tables[key].required) tables[key].required = []
    tables[key].required?.push(propertyName)
  }

  if (example) {
    const examples = (tables[key].examples as JSONSchemaArray) || [{}]
    switch (property.type) {
      case 'integer':
        ;(examples[0] as JSONSchemaObject)[propertyName] = parseInt(example as string)
        break
      case 'number':
        ;(examples[0] as JSONSchemaObject)[propertyName] = parseFloat(example as string)
        break
      case 'boolean':
        ;(examples[0] as JSONSchemaObject)[propertyName] = example === 'true'
        break
      default:
        ;(examples[0] as JSONSchemaObject)[propertyName] = example
        break
    }
    tables[key].examples = examples
  }
}

function transformPropertyName(propertyName?: string) {
  if (!propertyName) return ''
  propertyName = nameToKey(propertyName)
  // Lowercase the whole string if it is all caps
  if (propertyName === propertyName.toUpperCase()) return propertyName.toLowerCase()
  // otherwise, lowercase the first letter
  return propertyName[0].toLowerCase() + propertyName.slice(1).replace(/ /g, '')
}

export function extractPropertyName(objectReading: string[]) {
  const propertyNamePrefix = objectReading[0].split(' ')
  const propertyName = transformPropertyName(
    propertyNamePrefix
      .map((n) => (n === n.toUpperCase() ? n[0].toUpperCase() + n.slice(1).toLowerCase() : n))
      .join('') +
      objectReading
        .slice(1)
        .map((r) => r[0].toUpperCase() + r.slice(1))
        .join(''),
  )
  return propertyName
}
// #endregion

type Schema = JSONSchema & {
  nullable?: true | false
  properties?:
    | {
        [k: string]: Schema
      }
    | undefined
}
