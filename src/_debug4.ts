import { parseFORML2 } from './api/parse'
import { readFileSync } from 'fs'
const text = readFileSync('../support.auto.dev/domains/terms-of-service.md', 'utf-8')
const r = parseFORML2(text, [])
console.error('READINGS:', r.readings.length)
for (const rd of r.readings) {
  if (rd.text.startsWith('Each ') || rd.text.startsWith('It is ') || rd.text.startsWith('For each')) {
    console.error('  BAD: ' + rd.text.slice(0, 100))
  }
}
console.error('CONSTRAINTS:', r.constraints.length)
