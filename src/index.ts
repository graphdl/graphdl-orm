import { DurableObject } from 'cloudflare:workers'

export class GraphDLDB extends DurableObject {
  async fetch(): Promise<Response> {
    return new Response('not implemented', { status: 501 })
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    return new Response(JSON.stringify({ status: 'ok', version: '0.1.0' }), {
      headers: { 'Content-Type': 'application/json' },
    })
  },
}

export interface Env {
  GRAPHDL_DB: DurableObjectNamespace
  ENVIRONMENT: string
}
