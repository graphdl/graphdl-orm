import { mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'fs'
import { tmpdir } from 'os'
import { join, resolve } from 'path'
import { afterEach, describe, expect, it } from 'vitest'

import {
  appendManagedInstanceFacts,
  buildApplyInstanceFacts,
  buildAppCompileArgs,
  checkArestApps,
  createArestApp,
  inferInitialAppName,
  inspectArestApp,
  listArestApps,
  managedInstanceReadingPath,
  normalizeAppName,
  resolveArestApp,
} from './apps.js'

let tempDirs: string[] = []

function tempAppsDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'arest-mcp-apps-'))
  tempDirs.push(dir)
  return dir
}

afterEach(() => {
  for (const dir of tempDirs) rmSync(dir, { recursive: true, force: true })
  tempDirs = []
})

describe('MCP app registry helpers', () => {
  it('rejects names that can escape the apps directory', () => {
    expect(normalizeAppName('codex')).toBe('codex')
    expect(() => normalizeAppName('../codex')).toThrow(/Invalid AREST app name/)
    expect(() => normalizeAppName('codex/readings')).toThrow(/Invalid AREST app name/)
  })

  it('infers the initial app from explicit readings or db paths', () => {
    expect(inferInitialAppName({
      AREST_READINGS_DIR: 'C:/Users/lippe/Repos/apps/codex/readings',
    })).toBe('codex')
    expect(inferInitialAppName({
      AREST_DB: 'C:/Users/lippe/Repos/apps/support/support.db',
    })).toBe('support')
  })

  it('resolves an existing app and preserves its discovered db name', () => {
    const appsDir = tempAppsDir()
    const appRoot = join(appsDir, 'codex')
    mkdirSync(join(appRoot, 'readings'), { recursive: true })
    writeFileSync(join(appRoot, 'codex-arest.db'), '', 'utf8')

    const app = resolveArestApp('codex', { appsDir })

    expect(app).toMatchObject({
      name: 'codex',
      root: resolve(appRoot),
      readingsDir: resolve(appRoot, 'readings'),
      dbPath: resolve(appRoot, 'codex-arest.db'),
      exists: true,
      hasReadings: true,
      hasDb: true,
      hasAppMarker: false,
      source: 'apps_dir',
    })
  })

  it('creates app readings and lists apps', () => {
    const appsDir = tempAppsDir()
    const app = createArestApp('support', { appsDir }, 'Ticket(.Ticket Id) is an entity type.')

    expect(app.hasReadings).toBe(true)
    expect(app.hasDb).toBe(false)
    expect(listArestApps({ appsDir, libraryDirs: [] }).map((entry) => entry.name)).toEqual(['support'])
  })

  it('does not overwrite an existing initial reading', () => {
    const appsDir = tempAppsDir()
    createArestApp('support', { appsDir }, 'Ticket(.Ticket Id) is an entity type.')

    expect(() => createArestApp('support', { appsDir }, 'Ticket has Title.')).toThrow(/Initial reading already exists/)
  })

  it('writes managed instance facts under readings without changing schema readings', () => {
    const appsDir = tempAppsDir()
    const reading = 'Ticket(.Ticket Id) is an entity type.'
    const app = createArestApp('support', { appsDir }, reading)

    const result = appendManagedInstanceFacts(app.readingsDir, [
      "Ticket 't-1' has Title 'Lost password'.",
    ])

    const path = managedInstanceReadingPath(app.readingsDir)
    expect(result).toMatchObject({
      path,
      appended: 1,
      skipped: 0,
      lines: [
        "Ticket 't-1' has Title 'Lost password'.",
      ],
    })
    expect(readFileSync(path, 'utf8')).toContain('## Instance Facts')
    expect(readFileSync(path, 'utf8')).toContain("Ticket 't-1' has Title 'Lost password'.")
    expect(readFileSync(join(app.readingsDir, 'app.md'), 'utf8')).toBe(reading)

    const duplicate = appendManagedInstanceFacts(app.readingsDir, ["Ticket 't-1' has Title 'Lost password'."])
    expect(duplicate.appended).toBe(0)
    expect(duplicate.skipped).toBe(1)
  })

  it('builds apply instance facts without accepting unsafe FORML literals', () => {
    const built = buildApplyInstanceFacts({
      operation: 'create',
      noun: 'Support Ticket',
      id: 't-1',
      fields: {
        Title: 'Lost password',
        Priority: 'High',
      },
    })

    expect(built.lines).toEqual([
      "Support Ticket 't-1' has Priority 'High'.",
      "Support Ticket 't-1' has Title 'Lost password'.",
    ])
    expect(built.warnings).toEqual([])

    const unsafe = buildApplyInstanceFacts({
      operation: 'update',
      noun: 'Support Ticket',
      id: 't-1',
      fields: {
        Title: "O'Hare",
      },
    })
    expect(unsafe.lines).toEqual([])
    expect(unsafe.warnings).toContain("Support Ticket.Title is not safe to write as a single-quoted FORML literal: \"O'Hare\"")
    expect(unsafe.warnings).toContain("no safe instance field facts were generated for Support Ticket \"t-1\"")
  })

  it('reports an app that needs compilation when readings exist but the db is missing', () => {
    const appsDir = tempAppsDir()
    const app = createArestApp('support', { appsDir }, 'Ticket(.Ticket Id) is an entity type.')
    const inspected = inspectArestApp(app)

    expect(inspected.health.status).toBe('needs_compile')
    expect(inspected.health.issues).toContain('SQLite DB is missing')
    expect(inspected.health.readings.count).toBe(1)
    expect(inspected.health.next_actions).toContainEqual({
      tool: 'apps.compile',
      args: { name: 'support' },
      reason: 'compile readings into the app SQLite DB',
    })
  })

  it('reports readings-only roots without app markers or dbs as libraries', () => {
    const appsDir = tempAppsDir()
    const appRoot = join(appsDir, 'domain-lib')
    mkdirSync(join(appRoot, 'readings'), { recursive: true })
    writeFileSync(join(appRoot, 'readings', 'terms.md'), 'Term(.Name) is an entity type.', 'utf8')

    const inspected = inspectArestApp(resolveArestApp('domain-lib', { appsDir }))

    expect(inspected.health.status).toBe('library')
    expect(inspected.health.ok).toBe(true)
    expect(inspected.health.issues).toEqual([])
    expect(inspected.health.next_actions).toEqual([])
  })

  it('prefers app.db when multiple db files exist', () => {
    const appsDir = tempAppsDir()
    const appRoot = join(appsDir, 'support.auto.dev')
    mkdirSync(join(appRoot, 'readings'), { recursive: true })
    writeFileSync(join(appRoot, 'app.md'), '# Support App', 'utf8')
    writeFileSync(join(appRoot, 'app.db'), '', 'utf8')
    writeFileSync(join(appRoot, 'support.db'), '', 'utf8')

    const app = resolveArestApp('support.auto.dev', { appsDir })

    expect(app.dbPath).toBe(resolve(appRoot, 'app.db'))
    expect(app.hasDb).toBe(true)
    expect(app.hasAppMarker).toBe(true)
  })

  it('does not treat package kind as an app marker', () => {
    const root = tempAppsDir()
    const appsDir = join(root, 'apps')
    const euLaw = join(root, 'eu-law')
    mkdirSync(join(euLaw, 'readings'), { recursive: true })
    writeFileSync(join(euLaw, 'package.json'), JSON.stringify({
      name: 'arest-eu-law',
      kind: 'app',
    }), 'utf8')
    writeFileSync(join(euLaw, 'readings', 'gdpr.md'), 'Regulation(.Regulation Id) is an entity type.', 'utf8')

    const app = resolveArestApp('eu-law', { appsDir, libraryDirs: [root] })

    expect(app.hasAppMarker).toBe(false)
    expect(inspectArestApp(app).health.status).toBe('library')
  })

  it('reports stale DBs when readings are newer than the db', () => {
    const appsDir = tempAppsDir()
    const appRoot = join(appsDir, 'support')
    const readingsDir = join(appRoot, 'readings')
    const dbPath = join(appRoot, 'support.db')
    mkdirSync(readingsDir, { recursive: true })
    writeFileSync(join(readingsDir, 'app.md'), 'Ticket(.Ticket Id) is an entity type.', 'utf8')
    writeFileSync(dbPath, '', 'utf8')
    utimesSync(dbPath, new Date('2026-01-01T00:00:00Z'), new Date('2026-01-01T00:00:00Z'))
    utimesSync(join(readingsDir, 'app.md'), new Date('2026-01-02T00:00:00Z'), new Date('2026-01-02T00:00:00Z'))

    const inspected = inspectArestApp(resolveArestApp('support', { appsDir }))

    expect(inspected.health.status).toBe('stale_db')
    expect(inspected.health.db.stale).toBe(true)
  })

  it('reports stale DBs when dependency readings are newer than the db', () => {
    const root = tempAppsDir()
    const appsDir = join(root, 'apps')
    const supportRoot = join(appsDir, 'support')
    const autoDevRoot = join(appsDir, 'auto.dev')
    const lawCoreRoot = join(root, 'law-core')
    mkdirSync(join(supportRoot, 'readings'), { recursive: true })
    mkdirSync(join(autoDevRoot, 'readings'), { recursive: true })
    mkdirSync(join(lawCoreRoot, 'readings'), { recursive: true })
    writeFileSync(join(supportRoot, 'package.json'), JSON.stringify({
      name: 'arest-support',
      kind: 'app',
      dependencies: {
        'arest-auto-dev': 'file:../auto.dev',
      },
    }), 'utf8')
    writeFileSync(join(autoDevRoot, 'package.json'), JSON.stringify({
      name: 'arest-auto-dev',
      kind: 'library',
      dependencies: {
        'arest-law-core': 'file:../../law-core',
      },
    }), 'utf8')
    writeFileSync(join(lawCoreRoot, 'package.json'), JSON.stringify({
      name: 'arest-law-core',
      kind: 'library',
    }), 'utf8')
    writeFileSync(join(supportRoot, 'readings', 'app.md'), 'Ticket(.Ticket Id) is an entity type.', 'utf8')
    writeFileSync(join(autoDevRoot, 'readings', 'auto.md'), 'Plan(.Plan Id) is an entity type.', 'utf8')
    writeFileSync(join(lawCoreRoot, 'readings', 'core.md'), 'Jurisdiction(.Name) is an entity type.', 'utf8')
    writeFileSync(join(supportRoot, 'support.db'), '', 'utf8')
    utimesSync(join(supportRoot, 'readings', 'app.md'), new Date('2026-01-01T00:00:00Z'), new Date('2026-01-01T00:00:00Z'))
    utimesSync(join(autoDevRoot, 'readings', 'auto.md'), new Date('2026-01-02T00:00:00Z'), new Date('2026-01-02T00:00:00Z'))
    utimesSync(join(supportRoot, 'support.db'), new Date('2026-01-03T00:00:00Z'), new Date('2026-01-03T00:00:00Z'))
    utimesSync(join(lawCoreRoot, 'readings', 'core.md'), new Date('2026-01-04T00:00:00Z'), new Date('2026-01-04T00:00:00Z'))

    const inspected = inspectArestApp(resolveArestApp('support', { appsDir, libraryDirs: [root] }), { appsDir, libraryDirs: [root] })

    expect(inspected.health.status).toBe('stale_db')
    expect(inspected.health.issues).toContain('SQLite DB is older than one or more dependency readings')
    expect(inspected.health.dependencies.stale).toBe(true)
    expect(inspected.health.dependencies.direct.map((dependency) => dependency.name)).toEqual(['auto.dev'])
    expect(inspected.health.dependencies.closure.map((dependency) => dependency.name)).toEqual(['auto.dev', 'law-core'])
  })

  it('summarizes app health across the registry', () => {
    const appsDir = tempAppsDir()
    createArestApp('needs-compile', { appsDir }, 'Ticket(.Ticket Id) is an entity type.')
    const readyRoot = join(appsDir, 'ready')
    mkdirSync(join(readyRoot, 'readings'), { recursive: true })
    writeFileSync(join(readyRoot, 'readings', 'app.md'), 'Order(.Order Id) is an entity type.', 'utf8')
    writeFileSync(join(readyRoot, 'ready.db'), '', 'utf8')

    const check = checkArestApps({ appsDir, libraryDirs: [] })

    expect(check.summary.total).toBe(2)
    expect(check.summary.ready).toBe(1)
    expect(check.summary.needs_compile).toBe(1)
    expect(check.summary.issue_count).toBe(1)
  })

  it('does not count libraries as issues in registry health', () => {
    const appsDir = tempAppsDir()
    const appRoot = join(appsDir, 'domain-lib')
    mkdirSync(join(appRoot, 'readings'), { recursive: true })
    writeFileSync(join(appRoot, 'readings', 'terms.md'), 'Term(.Name) is an entity type.', 'utf8')

    const check = checkArestApps({ appsDir, libraryDirs: [] })

    expect(check.summary.total).toBe(1)
    expect(check.summary.library).toBe(1)
    expect(check.summary.issue_count).toBe(0)
  })

  describe('buildAppCompileArgs', () => {
    it('returns just own readings + --db when there are no dependencies', () => {
      const appsDir = tempAppsDir()
      createArestApp('solo', { appsDir }, 'Order(.Order Id) is an entity type.')
      const app = resolveArestApp('solo', { appsDir })
      const args = buildAppCompileArgs(app, { appsDir })
      expect(args).toEqual([app.readingsDir, '--db', app.dbPath])
    })

    it('prepends every dependency readings dir, deepest leaf first, before the app dir', () => {
      // Topology:
      //   support  ──depends on──>  auto.dev  ──depends on──>  law-core
      // Closure walk is pre-order: [auto.dev, law-core]. For arest-cli the
      // safest order is leaf-first: [law-core, auto.dev], then the app.
      const root = tempAppsDir()
      const appsDir = join(root, 'apps')
      const supportRoot = join(appsDir, 'support')
      const autoDevRoot = join(appsDir, 'auto.dev')
      const lawCoreRoot = join(root, 'law-core')
      mkdirSync(join(supportRoot, 'readings'), { recursive: true })
      mkdirSync(join(autoDevRoot, 'readings'), { recursive: true })
      mkdirSync(join(lawCoreRoot, 'readings'), { recursive: true })
      writeFileSync(join(supportRoot, 'package.json'), JSON.stringify({
        name: 'arest-support',
        kind: 'app',
        dependencies: { 'arest-auto-dev': 'file:../auto.dev' },
      }), 'utf8')
      writeFileSync(join(autoDevRoot, 'package.json'), JSON.stringify({
        name: 'arest-auto-dev',
        kind: 'library',
        dependencies: { 'arest-law-core': 'file:../../law-core' },
      }), 'utf8')
      writeFileSync(join(lawCoreRoot, 'package.json'), JSON.stringify({
        name: 'arest-law-core',
        kind: 'library',
      }), 'utf8')
      writeFileSync(join(supportRoot, 'readings', 'app.md'), 'Ticket(.Ticket Id) is an entity type.', 'utf8')
      writeFileSync(join(autoDevRoot, 'readings', 'auto.md'), 'Plan(.Plan Id) is an entity type.', 'utf8')
      writeFileSync(join(lawCoreRoot, 'readings', 'core.md'), 'Jurisdiction(.Name) is an entity type.', 'utf8')

      const support = resolveArestApp('support', { appsDir, libraryDirs: [root] })
      const args = buildAppCompileArgs(support, { appsDir, libraryDirs: [root] })

      expect(args).toEqual([
        resolve(lawCoreRoot, 'readings'),
        resolve(autoDevRoot, 'readings'),
        support.readingsDir,
        '--db',
        support.dbPath,
      ])
    })

    it('skips dependencies that have no readings or do not exist on disk', () => {
      const root = tempAppsDir()
      const appsDir = join(root, 'apps')
      const supportRoot = join(appsDir, 'support')
      mkdirSync(join(supportRoot, 'readings'), { recursive: true })
      writeFileSync(join(supportRoot, 'package.json'), JSON.stringify({
        name: 'arest-support',
        kind: 'app',
        dependencies: { 'arest-missing-lib': 'file:../missing-lib' },
      }), 'utf8')
      writeFileSync(join(supportRoot, 'readings', 'app.md'), 'Ticket(.Ticket Id) is an entity type.', 'utf8')

      const app = resolveArestApp('support', { appsDir, libraryDirs: [root] })
      const args = buildAppCompileArgs(app, { appsDir, libraryDirs: [root] })

      expect(args).toEqual([app.readingsDir, '--db', app.dbPath])
    })
  })

  it('discovers AREST package roots outside the apps directory', () => {
    const root = tempAppsDir()
    const appsDir = join(root, 'apps')
    mkdirSync(appsDir, { recursive: true })
    const lawCore = join(root, 'law-core')
    const usLaw = join(root, 'us-law')
    mkdirSync(join(lawCore, 'readings'), { recursive: true })
    mkdirSync(join(usLaw, 'readings', 'statutory'), { recursive: true })
    writeFileSync(join(lawCore, 'package.json'), JSON.stringify({
      name: 'arest-law-core',
      kind: 'library',
      description: 'Shared legal concepts',
    }), 'utf8')
    writeFileSync(join(lawCore, 'readings', 'core-types.md'), 'Jurisdiction(.Name) is an entity type.', 'utf8')
    writeFileSync(join(usLaw, 'package.json'), JSON.stringify({
      name: 'arest-us-law',
      kind: 'app',
      description: 'United States law',
    }), 'utf8')
    writeFileSync(join(usLaw, 'readings', 'app.md'), 'Law Matter(.Matter Id) is an entity type.', 'utf8')
    writeFileSync(join(usLaw, 'readings', 'statutory', 'fdcpa.md'), 'Debt Collector(.Name) is an entity type.', 'utf8')
    writeFileSync(join(usLaw, 'us-law.db'), '', 'utf8')

    const apps = listArestApps({ appsDir, libraryDirs: [root] })
    const core = apps.find((app) => app.name === 'law-core')
    const us = apps.find((app) => app.name === 'us-law')

    expect(core).toMatchObject({
      source: 'package_root',
      packageName: 'arest-law-core',
      packageKind: 'library',
    })
    expect(inspectArestApp(core!).health.status).toBe('library')
    expect(us).toMatchObject({
      source: 'package_root',
      packageName: 'arest-us-law',
      packageKind: 'app',
      hasAppMarker: true,
      dbPath: resolve(usLaw, 'us-law.db'),
    })
    expect(inspectArestApp(us!).health.readings.count).toBe(2)
  })
})
