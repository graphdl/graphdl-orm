import { it, expect } from 'vitest'
import { parseFORML2 } from './api/parse'
import fs from 'fs'

it('support.md validation: no bad nouns', () => {
  const text = fs.readFileSync('../support.auto.dev/domains/support.md', 'utf-8')
  const claims = parseFORML2(text, [])
  // Parser errors only — "Customer", "Request", "API" are legitimate cross-domain nouns
  const bad = claims.nouns.filter(n => ['At','Sent','No','Cross','Ring','Constraints','Product','Feature'].includes(n.name))
  console.log('Bad nouns:', bad.length === 0 ? 'NONE (clean!)' : bad.map(n => n.name))
  console.log('Total nouns:', claims.nouns.length)
  console.log('Total readings:', claims.readings.length)
  console.log('Total constraints:', claims.constraints.length)
  expect(bad).toHaveLength(0)
})
