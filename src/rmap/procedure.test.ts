import { describe, it, expect } from 'vitest'
import { rmap } from './procedure'

describe('RMAP Step 1: compound UC to separate table', () => {
  it('maps M:N binary to separate table with compound PK', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Language', objectType: 'entity', refScheme: 'code' },
      ],
      factTypes: [{
        id: 'ft1',
        reading: 'Person speaks Language',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Language', roleIndex: 1 }],
      }],
      constraints: [{
        kind: 'UC',
        factTypeId: 'ft1',
        roles: [0, 1], // spanning UC — compound key
      }],
    }
    const tables = rmap(schema)
    const speaksTable = tables.find(t => t.columns.some(c => c.name === 'person_id'))
    expect(speaksTable).toBeDefined()
    expect(speaksTable!.primaryKey).toEqual(['person_id', 'language_id'])
  })
})

describe('RMAP Step 2: functional roles grouped by entity', () => {
  it('groups functional fact types into entity table', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
        { name: 'Country', objectType: 'entity', refScheme: 'code' },
      ],
      factTypes: [
        { id: 'ft1', reading: 'Person has Name', roles: [
          { nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }
        ]},
        { id: 'ft2', reading: 'Person was born in Country', roles: [
          { nounName: 'Person', roleIndex: 0 }, { nounName: 'Country', roleIndex: 1 }
        ]},
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'UC', factTypeId: 'ft2', roles: [0] },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    expect(personTable!.columns.map(c => c.name)).toContain('name')
    expect(personTable!.columns.map(c => c.name)).toContain('country_id')
  })
})

describe('RMAP Step 6: constraint mapping', () => {
  it('maps MC to NOT NULL', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [{ id: 'ft1', reading: 'Person has Name', roles: [
        { nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }
      ]}],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'MC', factTypeId: 'ft1', roles: [0] }, // mandatory on Person role
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    const nameCol = personTable?.columns.find(c => c.name === 'name')
    expect(nameCol?.nullable).toBe(false)
  })
})

// ---------------------------------------------------------------------------
// New tests for Task 11
// ---------------------------------------------------------------------------

describe('RMAP Step 0.1: binarize exclusive unaries', () => {
  it('binarizes exclusive unaries into status column', () => {
    // Two unary facts on Person with XO constraint: "Person is male", "Person is female"
    // → single "sex" column with CHECK constraint
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [
        {
          id: 'ft_name', reading: 'Person has Name',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
        {
          id: 'ft_male', reading: 'Person is male',
          roles: [{ nounName: 'Person', roleIndex: 0 }],  // unary
        },
        {
          id: 'ft_female', reading: 'Person is female',
          roles: [{ nounName: 'Person', roleIndex: 0 }],  // unary
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft_name', roles: [0] },
        { kind: 'MC', factTypeId: 'ft_name', roles: [0] },
        // XO constraint across the two unary fact types
        { kind: 'XO', factTypeId: 'ft_male', roles: [0], xoGroup: ['ft_male', 'ft_female'] },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    // Should have a status column for the XO group
    const sexCol = personTable!.columns.find(c => c.name === 'sex')
    expect(sexCol).toBeDefined()
    expect(sexCol!.type).toBe('TEXT')
    // The CHECK clause lists the allowed values
    expect(personTable!.checks).toBeDefined()
    const sexCheck = personTable!.checks!.find(ch => ch.includes('sex'))
    expect(sexCheck).toBeDefined()
    expect(sexCheck).toContain('male')
    expect(sexCheck).toContain('female')
  })

  it('binarizes exclusive unaries as nullable when not mandatory', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [
        {
          id: 'ft_name', reading: 'Person has Name',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
        {
          id: 'ft_active', reading: 'Person is active',
          roles: [{ nounName: 'Person', roleIndex: 0 }],
        },
        {
          id: 'ft_inactive', reading: 'Person is inactive',
          roles: [{ nounName: 'Person', roleIndex: 0 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft_name', roles: [0] },
        { kind: 'MC', factTypeId: 'ft_name', roles: [0] },
        // XO but no MC on the unaries → nullable
        { kind: 'XO', factTypeId: 'ft_active', roles: [0], xoGroup: ['ft_active', 'ft_inactive'] },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    const statusCol = personTable!.columns.find(c => c.name === 'status')
    expect(statusCol).toBeDefined()
    expect(statusCol!.nullable).toBe(true)
  })
})

describe('RMAP Step 0.3: subtype absorption', () => {
  it('absorbs subtype columns into supertype table', () => {
    // Male and Female are subtypes of Person; Male has BeardLength, Female has MaidenName
    // → Person table absorbs subtype-specific columns as nullable
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
        { name: 'Male', objectType: 'entity' },
        { name: 'Female', objectType: 'entity' },
        { name: 'BeardLength', objectType: 'value' },
        { name: 'MaidenName', objectType: 'value' },
      ],
      factTypes: [
        {
          id: 'ft1', reading: 'Person has Name',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
        {
          id: 'ft2', reading: 'Male has BeardLength',
          roles: [{ nounName: 'Male', roleIndex: 0 }, { nounName: 'BeardLength', roleIndex: 1 }],
        },
        {
          id: 'ft3', reading: 'Female has MaidenName',
          roles: [{ nounName: 'Female', roleIndex: 0 }, { nounName: 'MaidenName', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'MC', factTypeId: 'ft1', roles: [0] },
        { kind: 'UC', factTypeId: 'ft2', roles: [0] },
        { kind: 'MC', factTypeId: 'ft2', roles: [0] },
        { kind: 'UC', factTypeId: 'ft3', roles: [0] },
        { kind: 'MC', factTypeId: 'ft3', roles: [0] },
      ],
      subtypes: [
        { subtype: 'Male', supertype: 'Person' },
        { subtype: 'Female', supertype: 'Person' },
      ],
    }
    const tables = rmap(schema)
    // Should have a single Person table, NOT separate Male/Female tables
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    expect(tables.find(t => t.name === 'male')).toBeUndefined()
    expect(tables.find(t => t.name === 'female')).toBeUndefined()

    const colNames = personTable!.columns.map(c => c.name)
    expect(colNames).toContain('name')          // from Person
    expect(colNames).toContain('beard_length')   // from Male (absorbed)
    expect(colNames).toContain('maiden_name')    // from Female (absorbed)

    // Subtype-specific columns must be nullable (not all Persons are Male/Female)
    const beardCol = personTable!.columns.find(c => c.name === 'beard_length')
    expect(beardCol!.nullable).toBe(true)
    const maidenCol = personTable!.columns.find(c => c.name === 'maiden_name')
    expect(maidenCol!.nullable).toBe(true)
  })

  it('follows transitive subtype chains to root', () => {
    // Postgrad is subtype of Student, Student is subtype of Person
    // → Postgrad columns absorbed into Person
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
        { name: 'Student', objectType: 'entity' },
        { name: 'Postgrad', objectType: 'entity' },
        { name: 'Thesis', objectType: 'value' },
      ],
      factTypes: [
        {
          id: 'ft1', reading: 'Person has Name',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
        {
          id: 'ft2', reading: 'Postgrad has Thesis',
          roles: [{ nounName: 'Postgrad', roleIndex: 0 }, { nounName: 'Thesis', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'MC', factTypeId: 'ft1', roles: [0] },
        { kind: 'UC', factTypeId: 'ft2', roles: [0] },
      ],
      subtypes: [
        { subtype: 'Student', supertype: 'Person' },
        { subtype: 'Postgrad', supertype: 'Student' },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    expect(tables.find(t => t.name === 'postgrad')).toBeUndefined()
    expect(tables.find(t => t.name === 'student')).toBeUndefined()
    expect(personTable!.columns.map(c => c.name)).toContain('thesis')
  })
})

describe('RMAP Step 3: 1:1 absorption', () => {
  it('absorbs 1:1 into table with fewer nulls', () => {
    // Person has Passport (1:1) — Person is mandatory for Passport
    // UC on both sides, MC on Person side → absorb passport_id into Person
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
        { name: 'Passport', objectType: 'entity', refScheme: 'number' },
      ],
      factTypes: [
        {
          id: 'ft_name', reading: 'Person has Name',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
        {
          id: 'ft_passport', reading: 'Person has Passport',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Passport', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft_name', roles: [0] },
        { kind: 'MC', factTypeId: 'ft_name', roles: [0] },
        // 1:1 — UC on each role separately
        { kind: 'UC', factTypeId: 'ft_passport', roles: [0] },
        { kind: 'UC', factTypeId: 'ft_passport', roles: [1] },
        // Person side is mandatory (every person has a passport)
        { kind: 'MC', factTypeId: 'ft_passport', roles: [0] },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    // passport_id absorbed into Person (since Person is mandatory = fewer nulls)
    expect(personTable!.columns.map(c => c.name)).toContain('passport_id')
    const passportCol = personTable!.columns.find(c => c.name === 'passport_id')
    expect(passportCol!.references).toBe('passport')
    expect(passportCol!.nullable).toBe(false)
    // No separate junction table for 1:1
    expect(tables.find(t => t.name === 'person_passport')).toBeUndefined()
  })

  it('absorbs 1:1 toward the mandatory side when only one side is mandatory', () => {
    // Employee has Parking-spot, MC on Employee side only (not all employees have spots)
    // → absorb parking_spot_id into Employee as nullable
    const schema = {
      nouns: [
        { name: 'Employee', objectType: 'entity', refScheme: 'id' },
        { name: 'ParkingSpot', objectType: 'entity', refScheme: 'number' },
      ],
      factTypes: [
        {
          id: 'ft1', reading: 'Employee has ParkingSpot',
          roles: [{ nounName: 'Employee', roleIndex: 0 }, { nounName: 'ParkingSpot', roleIndex: 1 }],
        },
      ],
      constraints: [
        // 1:1
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'UC', factTypeId: 'ft1', roles: [1] },
        // Only ParkingSpot side is mandatory (every parking spot is assigned)
        { kind: 'MC', factTypeId: 'ft1', roles: [1] },
      ],
    }
    const tables = rmap(schema)
    // Absorbed into ParkingSpot (the mandatory side)
    const spotTable = tables.find(t => t.name === 'parking_spot')
    expect(spotTable).toBeDefined()
    expect(spotTable!.columns.map(c => c.name)).toContain('employee_id')
    // Employee gets a table from Step 4 (independent entity) since it has no functional columns
  })
})

describe('RMAP Step 4: independent entity', () => {
  it('creates single-column table for independent entity', () => {
    // Color entity referenced by Product but has no own attributes
    const schema = {
      nouns: [
        { name: 'Product', objectType: 'entity', refScheme: 'code' },
        { name: 'Color', objectType: 'entity', refScheme: 'name' },
      ],
      factTypes: [
        {
          id: 'ft1', reading: 'Product has Color',
          roles: [{ nounName: 'Product', roleIndex: 0 }, { nounName: 'Color', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'MC', factTypeId: 'ft1', roles: [0] },
      ],
    }
    const tables = rmap(schema)
    const colorTable = tables.find(t => t.name === 'color')
    expect(colorTable).toBeDefined()
    expect(colorTable!.columns).toHaveLength(1)
    expect(colorTable!.columns[0].name).toBe('id')
    expect(colorTable!.primaryKey).toEqual(['id'])
  })

  it('does not duplicate entity that already has a table from Step 2', () => {
    // Product already has functional facts → table from Step 2
    // Should not emit a duplicate from Step 4
    const schema = {
      nouns: [
        { name: 'Product', objectType: 'entity', refScheme: 'code' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [
        {
          id: 'ft1', reading: 'Product has Name',
          roles: [{ nounName: 'Product', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
      ],
    }
    const tables = rmap(schema)
    const productTables = tables.filter(t => t.name === 'product')
    expect(productTables).toHaveLength(1)
  })
})

describe('RMAP Step 6 extensions: value constraints and FK references', () => {
  it('maps value constraints to CHECK clauses', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Rating', objectType: 'value' },
      ],
      factTypes: [
        {
          id: 'ft1', reading: 'Person has Rating',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Rating', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'MC', factTypeId: 'ft1', roles: [0] },
        { kind: 'VC', factTypeId: 'ft1', roles: [1], values: ['A', 'B', 'C'] },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    expect(personTable!.checks).toBeDefined()
    const check = personTable!.checks!.find(ch => ch.includes('rating'))
    expect(check).toBeDefined()
    expect(check).toContain("'A'")
    expect(check).toContain("'B'")
    expect(check).toContain("'C'")
  })

  it('maps subset constraints to FK references', () => {
    // "If some Person teaches Course then that Person studies Course" (SS)
    // The subset side (teaches) should reference the superset side (studies)
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
        { name: 'Course', objectType: 'entity', refScheme: 'code' },
      ],
      factTypes: [
        {
          id: 'ft_name', reading: 'Person has Name',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
        },
        {
          id: 'ft_teaches', reading: 'Person teaches Course',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Course', roleIndex: 1 }],
        },
        {
          id: 'ft_studies', reading: 'Person studies Course',
          roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Course', roleIndex: 1 }],
        },
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft_name', roles: [0] },
        { kind: 'MC', factTypeId: 'ft_name', roles: [0] },
        { kind: 'UC', factTypeId: 'ft_teaches', roles: [0, 1] },
        { kind: 'UC', factTypeId: 'ft_studies', roles: [0, 1] },
        // Subset: teaches ⊆ studies
        {
          kind: 'SS',
          factTypeId: 'ft_teaches',
          roles: [0, 1],
          targetFactTypeId: 'ft_studies',
          targetRoles: [0, 1],
        },
      ],
    }
    const tables = rmap(schema)
    const teachesTable = tables.find(t =>
      t.columns.some(c => c.name === 'person_id') &&
      t.name.includes('teach')
    )
    expect(teachesTable).toBeDefined()
    // The teaches table should have a FK reference annotation
    expect(teachesTable!.checks).toBeDefined()
    const ssCheck = teachesTable!.checks!.find(ch => ch.includes('FK'))
    expect(ssCheck).toBeDefined()
    expect(ssCheck).toContain('person_studies')
  })
})
