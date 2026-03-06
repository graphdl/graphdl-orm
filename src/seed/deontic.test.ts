import { describe, it, expect } from 'vitest'
import { groupDeonticInstances } from './deontic'
import type { DeonticConstraintInstanceDef } from './parser'

describe('groupDeonticInstances', () => {
  it('groups instance facts by constraint text correctly', () => {
    const constraints = [
      'It is obligatory that each Order has a Status',
      'It is forbidden that a Customer places more than 5 Orders per day',
    ]
    const instances: DeonticConstraintInstanceDef[] = [
      { constraint: 'It is obligatory that each Order has a Status', instance: 'Order #123 has Status "pending"' },
      { constraint: 'It is obligatory that each Order has a Status', instance: 'Order #456 has Status "shipped"' },
      { constraint: 'It is forbidden that a Customer places more than 5 Orders per day', instance: 'Customer "Alice" placed 3 Orders on 2026-03-06' },
    ]

    const result = groupDeonticInstances(constraints, instances)

    expect(result).toEqual([
      {
        constraintText: 'It is obligatory that each Order has a Status',
        instances: [
          'Order #123 has Status "pending"',
          'Order #456 has Status "shipped"',
        ],
      },
      {
        constraintText: 'It is forbidden that a Customer places more than 5 Orders per day',
        instances: [
          'Customer "Alice" placed 3 Orders on 2026-03-06',
        ],
      },
    ])
  })

  it('returns empty instances for constraints with no instance facts', () => {
    const constraints = [
      'It is obligatory that each Order has a Status',
      'It is forbidden that a Product has a negative Price',
    ]
    const instances: DeonticConstraintInstanceDef[] = [
      { constraint: 'It is obligatory that each Order has a Status', instance: 'Order #123 has Status "pending"' },
    ]

    const result = groupDeonticInstances(constraints, instances)

    expect(result).toHaveLength(2)
    expect(result[0].instances).toHaveLength(1)
    expect(result[1]).toEqual({
      constraintText: 'It is forbidden that a Product has a negative Price',
      instances: [],
    })
  })

  it('handles empty inputs', () => {
    expect(groupDeonticInstances([], [])).toEqual([])
    expect(groupDeonticInstances([], [{ constraint: 'x', instance: 'y' }])).toEqual([])
    expect(groupDeonticInstances(['x'], [])).toEqual([{ constraintText: 'x', instances: [] }])
  })
})
