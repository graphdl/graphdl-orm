import { Config } from './payload-types';
import * as gdl from './payload-types';
export declare function SDK(apiKey?: string): {
    api: import("payload-rest-client/dist/types").RPC<Config, "en">;
    things: import("payload-rest-client/dist/types").CollectionsApi<gdl.Thing, "en">;
    nouns: import("payload-rest-client/dist/types").CollectionsApi<gdl.Noun, "en">;
    resources: import("payload-rest-client/dist/types").CollectionsApi<gdl.Resource, "en">;
    verbs: import("payload-rest-client/dist/types").CollectionsApi<gdl.Verb, "en">;
    actions: import("payload-rest-client/dist/types").CollectionsApi<gdl.Action, "en">;
    constraints: import("payload-rest-client/dist/types").CollectionsApi<gdl.Constraint, "en">;
    roles: import("payload-rest-client/dist/types").CollectionsApi<gdl.Role, "en">;
    graphSchemas: import("payload-rest-client/dist/types").CollectionsApi<gdl.GraphSchema, "en">;
    graphs: import("payload-rest-client/dist/types").CollectionsApi<gdl.Graph, "en">;
    eventTypes: import("payload-rest-client/dist/types").CollectionsApi<gdl.EventType, "en">;
    events: import("payload-rest-client/dist/types").CollectionsApi<gdl.Event, "en">;
    data: import("payload-rest-client/dist/types").CollectionsApi<gdl.Datum, "en">;
    streams: import("payload-rest-client/dist/types").CollectionsApi<gdl.Stream, "en">;
    states: import("payload-rest-client/dist/types").CollectionsApi<gdl.Status, "en">;
    stateMachineDefinitions: import("payload-rest-client/dist/types").CollectionsApi<gdl.StateMachineDefinition, "en">;
    stateMachines: import("payload-rest-client/dist/types").CollectionsApi<gdl.StateMachine, "en">;
    transitions: import("payload-rest-client/dist/types").CollectionsApi<gdl.Transition, "en">;
    guardExpressionRunTypes: import("payload-rest-client/dist/types").CollectionsApi<gdl.GuardExpression, "en">;
    guardExpressionRuns: import("payload-rest-client/dist/types").CollectionsApi<gdl.GuardExpressionRun, "en">;
    relationalMap: (schemas: gdl.GraphSchema[]) => Promise<Record<string, {
        name: string;
        columns: gdl.Role[];
    }>>;
} | null;
export * from './payload-types';
//# sourceMappingURL=index.d.ts.map