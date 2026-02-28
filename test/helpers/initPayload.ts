import { getPayload, type Payload } from 'payload'

let cachedPayload: Payload | null = null

export async function initPayload(): Promise<Payload> {
  if (cachedPayload) return cachedPayload

  const { default: config } = await import('../../src/payload.config')
  cachedPayload = await getPayload({ config })
  return cachedPayload
}
