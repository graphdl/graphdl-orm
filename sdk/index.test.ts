import { expect, test } from 'vitest'
import { SDK } from './dist/index.js'

test('should return the sdk', () => {
  const sdk = SDK('test')
  expect(sdk).toBeTruthy()
})
