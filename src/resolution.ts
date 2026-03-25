export interface RegistryStub {
  resolveNoun(nounName: string): Promise<{ domainSlug: string; domainDoId: string } | null>
}

/**
 * Walk the registry chain to resolve a noun.
 * Registries are ordered by priority: [org, public].
 * First match wins — short-circuits.
 */
export async function resolveNounInChain(
  nounName: string,
  registries: RegistryStub[],
): Promise<{ domainSlug: string; domainDoId: string; registryIndex: number } | null> {
  for (let i = 0; i < registries.length; i++) {
    const result = await registries[i].resolveNoun(nounName)
    if (result) return { ...result, registryIndex: i }
  }
  return null
}
