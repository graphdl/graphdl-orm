/**
 * Claims Module — deterministic claim extraction and ingestion.
 *
 * This is the single open-source entry point for transforming structured claims
 * (nouns, readings, constraints, subtypes, transitions, instance facts) into
 * ORM entities in the Payload database.
 *
 * Three layers:
 * - tokenize: pure noun tokenization (no Payload dependency)
 * - constraints: pure multiplicity parsing + Payload constraint creation
 * - ingest: full ingestion orchestration (single reading or bulk claims)
 */

// Tokenization
export type { NounRef, TokenizeResult } from './tokenize'
export { tokenizeReading } from './tokenize'

// Constraints
export type { ConstraintDef } from './constraints'
export { parseMultiplicity, applyConstraints } from './constraints'

// Ingestion
export type {
  ExtractedClaims,
  IngestReadingResult,
  IngestClaimsResult,
} from './ingest'
export { ingestReading, ingestClaims } from './ingest'

// Converters (parser output -> ExtractedClaims)
export { domainParseToClaims, stateMachineParseToClaims, readingDefsToClaims } from './converters'
