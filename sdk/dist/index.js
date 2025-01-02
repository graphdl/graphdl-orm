"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __exportStar = (this && this.__exportStar) || function(m, exports) {
    for (var p in m) if (p !== "default" && !Object.prototype.hasOwnProperty.call(exports, p)) __createBinding(exports, m, p);
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.SDK = void 0;
const payload_rest_client_1 = require("payload-rest-client");
const dotenv_1 = require("dotenv");
(0, dotenv_1.config)();
const apiUrl = 'https://graphdl.org/api';
function SDK(apiKey) {
    if (!apiKey) {
        apiKey = process.env.GRAPHDL_API_KEY || '';
        if (!apiKey)
            return null;
    }
    const api = (0, payload_rest_client_1.createClient)({
        apiUrl,
        headers: {
            Authorization: `users API-Key ${apiKey}`,
        },
        cache: 'no-store',
    });
    async function graphSchema(o) {
        let schema;
        if (typeof o === 'string') {
            schema = await api.collections['graph-schemas'].findById({ id: o });
        }
        else
            schema = o;
        schema.subject = await role(schema.subject);
        if (schema.verb)
            schema.verb = await verb(schema.verb);
        if (schema.roles)
            schema.roles = await Promise.all(schema.roles.map(role));
        return schema;
    }
    async function role(o) {
        let role;
        if (typeof o === 'string') {
            role = await api.collections.roles.findById({ id: o });
        }
        else {
            role = o;
        }
        if (role.constraints)
            role.constraints = await Promise.all(role.constraints.map(constraint));
        if (role.noun?.relationTo === 'nouns')
            role.noun.value = await noun(role.noun.value);
        else if (role.noun?.relationTo === 'graph-schemas')
            role.noun.value = await graphSchema(role.noun.value);
        return role;
    }
    async function constraint(o) {
        if (typeof o === 'string') {
            return await api.collections.constraints.findById({ id: o });
        }
        else {
            return o;
        }
    }
    async function noun(o) {
        if (typeof o === 'string') {
            return await api.collections.nouns.findById({ id: o });
        }
        else {
            return o;
        }
    }
    async function verb(o) {
        if (typeof o === 'string') {
            return await api.collections.verbs.findById({ id: o });
        }
        else {
            return o;
        }
    }
    function predicate(o) {
        return [o.subject, ...(o.roles ? o.roles : [])];
    }
    return {
        api,
        things: api.collections.things,
        nouns: api.collections.nouns,
        resources: api.collections.resources,
        verbs: api.collections.verbs,
        actions: api.collections.actions,
        constraints: api.collections.constraints,
        roles: api.collections.roles,
        graphSchemas: api.collections['graph-schemas'],
        graphs: api.collections.graphs,
        eventTypes: api.collections['event-types'],
        events: api.collections.events,
        data: api.collections.data,
        streams: api.collections.streams,
        states: api.collections.statuses,
        stateMachineDefinitions: api.collections['state-machine-definitions'],
        stateMachines: api.collections['state-machines'],
        transitions: api.collections.transitions,
        guardExpressionRunTypes: api.collections['guard-expressions'],
        guardExpressionRuns: api.collections['guard-expression-runs'],
        relationalMap: async (schemas) => {
            schemas = await Promise.all(schemas.map(graphSchema));
            let tables = {};
            const compoundUniqueSchemas = schemas.filter((schema) => {
                // check duplicate constraints by id to find composite uniqueness schemas
                const ucs = predicate(schema)
                    // get constraints
                    .flatMap((r) => (r.constraints ? r.constraints : []))
                    // filter to uniqueness constraints
                    .filter((c) => c.kind === 'UC')
                    .map((c) => c.id);
                return ucs.some((uc) => ucs.filter((c) => c === uc).length > 1);
            });
            compoundUniqueSchemas.forEach((schema) => {
                tables[schema.id] = { name: schema.name || 'table' + Object.keys(tables).length, columns: predicate(schema) };
            });
            const functionalSchemas = schemas.filter((schema) => {
                const ucs = predicate(schema)
                    // get constraints
                    .flatMap((r) => (r.constraints ? r.constraints : []))
                    // filter to uniqueness constraints
                    .filter((c) => c.kind === 'UC')
                    .map((c) => c.id);
                return ucs.some((uc) => ucs.filter((c) => c === uc).length === 1);
            });
            functionalSchemas.forEach((schema) => {
                const functionalRole = predicate(schema).find((r) => r.constraints?.find((c) => c.kind === 'UC'));
                const nounId = functionalRole.noun?.value?.id;
                if (!tables[nounId])
                    tables[nounId] = { name: schema.name || 'table' + Object.keys(tables).length, columns: [] };
                tables[nounId].columns.push(...predicate(schema));
            });
            const allRoles = schemas.flatMap((schema) => predicate(schema));
            const independentNouns = allRoles.filter((r) => (!r.constraints || r.constraints.every((c) => c.kind !== 'UC')) &&
                allRoles
                    .filter((r2) => r2.noun?.value?.id === r.noun?.value?.id)
                    .every((r) => !r.constraints || r.constraints.every((c) => c.kind !== 'UC')));
            independentNouns.forEach((noun) => {
                const nounId = noun.noun?.value?.id;
                if (!tables[nounId])
                    tables[nounId] = { name: noun.name || 'table' + Object.keys(tables).length, columns: [] };
                tables[nounId].columns.push(noun);
            });
            return tables;
        },
    };
}
exports.SDK = SDK;
__exportStar(require("./payload-types"), exports);
//# sourceMappingURL=index.js.map