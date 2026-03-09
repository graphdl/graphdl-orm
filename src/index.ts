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
