/**
 * DO adapter — bridges step functions to DomainDB RPC.
 * The step functions expect a db with findInCollection/createInCollection/updateInCollection.
 * This adapter implements that interface, delegating to any object with those methods.
 */

export interface GraphDLDBLike {
  findInCollection(collection: string, where: any, opts?: any): Promise<{ docs: any[]; totalDocs: number }>
  createInCollection(collection: string, data: any): Promise<any>
  updateInCollection(collection: string, id: string, updates: any): Promise<any>
}

export function createDomainAdapter(target: GraphDLDBLike): GraphDLDBLike {
  return {
    findInCollection: (collection, where, opts) => target.findInCollection(collection, where, opts),
    createInCollection: (collection, data) => target.createInCollection(collection, data),
    updateInCollection: (collection, id, updates) => target.updateInCollection(collection, id, updates),
  }
}
