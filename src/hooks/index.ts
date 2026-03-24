/**
 * Collection write hooks — DEPRECATED.
 *
 * The hook functions (nounAfterCreate, readingAfterCreate, constraintAfterCreate,
 * smDefinitionAfterCreate) have been superseded by the BatchBuilder-based step
 * functions in src/claims/steps.ts. Those functions accumulate metamodel entities
 * synchronously in memory rather than issuing async DB calls.
 *
 * The parse-constraint module remains in this directory as a pure-function
 * parser used by the /api/parse endpoint and the claims pipeline.
 */

export { parseConstraintText, parseSetComparisonBlock } from './parse-constraint'
