/**
 * Service chain tests — #18 service-health, #19 incident response, #20 deontic loop.
 *
 * Tests that the service-health, incident, and support response readings
 * parse correctly through the WASM engine and that state machines,
 * derivation rules, and deontic constraints are recognized.
 */

import { describe, it, expect } from 'vitest'
import { readFileSync } from 'fs'
import { resolve } from 'path'

// These tests verify the readings exist and contain the expected patterns.
// Full WASM parsing is tested in the Rust unit tests.

const serviceHealthPath = resolve(__dirname, '../../../support.auto.dev/readings/service-health.md')
// __dirname = graphdl-orm/src/api, ../../../ = Repos/
const serviceHealthReadings = readFileSync(serviceHealthPath, 'utf-8').replace(/\r\n/g, '\n')

describe('Service Health (#18)', () => {
  it('declares entity types for monitoring', () => {
    expect(serviceHealthReadings).toContain('Log Entry(.id) is an entity type.')
    expect(serviceHealthReadings).toContain('Service Health(.id) is an entity type.')
    expect(serviceHealthReadings).toContain('Incident(.id) is an entity type.')
  })

  it('declares health status value type with valid values', () => {
    expect(serviceHealthReadings).toContain("The possible values of Service Health Status are 'healthy', 'degraded', 'down'.")
  })

  it('declares incident status value type', () => {
    expect(serviceHealthReadings).toContain("The possible values of Incident Status are 'open', 'investigating', 'escalated', 'resolved'.")
  })

  it('has derivation rules for health status', () => {
    expect(serviceHealthReadings).toContain("External System has Service Health Status 'degraded' if Error Rate")
    expect(serviceHealthReadings).toContain("External System has Service Health Status 'down' if Error Rate")
    expect(serviceHealthReadings).toContain("External System has Service Health Status 'healthy' if Error Rate")
  })

  it('has derivation rule for incident creation', () => {
    expect(serviceHealthReadings).toContain("Incident is created if External System has Service Health Status 'degraded'")
  })

  it('declares incident lifecycle state machine', () => {
    expect(serviceHealthReadings).toContain("Incident Status 'open' is initial in State Machine Definition 'Incident'.")
    expect(serviceHealthReadings).toContain("Transition 'investigate' is from Incident Status 'open'.")
    expect(serviceHealthReadings).toContain("Transition 'resolve' is from Incident Status 'investigating'.")
    expect(serviceHealthReadings).toContain("Transition 'resolve' is from Incident Status 'escalated'.")
  })
})

describe('Incident Response (#19)', () => {
  it('has investigation action rules', () => {
    expect(serviceHealthReadings).toContain("Incident investigation reads logs if Incident has Incident Status 'investigating'.")
    expect(serviceHealthReadings).toContain('Incident fix is attempted via reading update if investigation finds external model change.')
    expect(serviceHealthReadings).toContain('Incident fix is attempted via code patch if investigation finds code bug in existing product.')
  })

  it('has escalation rules', () => {
    expect(serviceHealthReadings).toContain("Incident transitions to 'escalated' if fix attempt fails.")
    expect(serviceHealthReadings).toContain("Incident transitions to 'escalated' if root cause is undetermined.")
  })
})

describe('Support Response Deontic Loop (#20)', () => {
  it('has deontic permissions for autonomous actions', () => {
    expect(serviceHealthReadings).toContain('It is permitted that retry is executed autonomously.')
    expect(serviceHealthReadings).toContain('It is permitted that fallback is executed autonomously.')
    expect(serviceHealthReadings).toContain('It is permitted that reading update is executed autonomously.')
  })

  it('has deontic obligations for human oversight', () => {
    expect(serviceHealthReadings).toContain('It is obligatory that code change requires Approval.')
    expect(serviceHealthReadings).toContain("It is obligatory that Escalation Notification is sent when Incident has Incident Status 'escalated'.")
  })

  it('has deontic prohibitions for safety', () => {
    expect(serviceHealthReadings).toContain('It is forbidden that autonomous action modifies Plan.')
    expect(serviceHealthReadings).toContain('It is forbidden that autonomous action deploys to production without verification.')
  })
})
