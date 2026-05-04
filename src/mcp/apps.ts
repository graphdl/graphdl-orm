import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'fs'
import { basename, dirname, join, relative, resolve } from 'path'

export interface ArestApp {
  name: string
  root: string
  readingsDir: string
  dbPath: string
  exists: boolean
  hasReadings: boolean
  hasDb: boolean
  hasAppMarker: boolean
  source: 'apps_dir' | 'package_root'
  packageName?: string
  packageKind?: 'app' | 'library' | string
  packageDescription?: string
  packageDependencies?: Record<string, string>
}

export interface AppRegistryOptions {
  appsDir?: string
  libraryDirs?: string[]
  cwd?: string
  explicitAppName?: string
  explicitReadingsDir?: string
  explicitDbPath?: string
}

export type ArestAppHealthStatus =
  | 'not_found'
  | 'library'
  | 'missing_readings'
  | 'needs_compile'
  | 'stale_db'
  | 'ready'

export interface ArestAppFileInfo {
  name: string
  path: string
  bytes: number
  modifiedMs: number
  modified: string
}

export interface ArestAppNextAction {
  tool: string
  reason: string
  args?: Record<string, unknown>
}

export interface ArestAppDependency {
  name: string
  root: string
  readingsDir: string
  packageName?: string
  packageKind?: 'app' | 'library' | string
  packageSpec?: string
  exists: boolean
  hasReadings: boolean
  source: 'apps_dir' | 'package_root'
  readings: {
    count: number
    newestModified: string | null
    newestModifiedMs: number | null
  }
}

export interface ArestAppHealth {
  status: ArestAppHealthStatus
  ok: boolean
  issues: string[]
  next_actions: ArestAppNextAction[]
  readings: {
    dir: string
    exists: boolean
    count: number
    newestModified: string | null
    newestModifiedMs: number | null
    files: ArestAppFileInfo[]
  }
  db: {
    path: string
    exists: boolean
    bytes: number | null
    modified: string | null
    modifiedMs: number | null
    stale: boolean
  }
  dependencies: {
    direct: ArestAppDependency[]
    closure: ArestAppDependency[]
    newestModified: string | null
    newestModifiedMs: number | null
    stale: boolean
  }
}

export interface ArestAppInspection extends ArestApp {
  health: ArestAppHealth
}

export interface ArestAppsCheck {
  summary: {
    total: number
    ready: number
    library: number
    not_found: number
    missing_readings: number
    needs_compile: number
    stale_db: number
    issue_count: number
  }
  apps: ArestAppInspection[]
}

export interface ApplyInstanceFactInput {
  operation: string
  noun: string
  id?: string
  fields?: Record<string, string>
  fieldFactTypes?: Record<string, ManagedInstanceFactTypeReading>
}

export interface ManagedInstanceFactTypeReading {
  reading: string
  roles: string[]
}

export interface ManagedInstanceFactBuildResult {
  lines: string[]
  warnings: string[]
}

export interface ManagedInstanceFactAppendResult {
  path: string
  appended: number
  skipped: number
  lines: string[]
}

const MANAGED_INSTANCE_READING_HEADER = `# MCP Managed Instance Facts

## Instance Facts
`

const SAFE_FORML_TERM = /^[A-Za-z][A-Za-z0-9 _-]*$/

export function normalizeAppName(name: string): string {
  const trimmed = name.trim()
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(trimmed)) {
    throw new Error(`Invalid AREST app name: ${name}`)
  }
  if (trimmed === '.' || trimmed === '..' || trimmed.includes('/') || trimmed.includes('\\')) {
    throw new Error(`Invalid AREST app name: ${name}`)
  }
  return trimmed
}

export function inferInitialAppName(env: NodeJS.ProcessEnv = process.env): string {
  if (env.AREST_APP) return normalizeAppName(env.AREST_APP)

  if (env.AREST_READINGS_DIR) {
    const readings = resolve(env.AREST_READINGS_DIR)
    const appRoot = basename(readings) === 'readings' ? dirname(readings) : readings
    return normalizeAppName(basename(appRoot))
  }

  if (env.AREST_DB) {
    return normalizeAppName(basename(dirname(resolve(env.AREST_DB))))
  }

  return 'default'
}

export function defaultAppsDir(cwd = process.cwd(), env: NodeJS.ProcessEnv = process.env): string {
  return resolve(env.AREST_APPS_DIR || join(cwd, '..', 'apps'))
}

export function resolveArestApp(name: string, options: AppRegistryOptions = {}): ArestApp {
  const appName = normalizeAppName(name)
  const appsDir = resolve(options.appsDir || defaultAppsDir(options.cwd))
  const appsRoot = resolve(appsDir, appName)
  const packageRoot = isDirectory(appsRoot) ? null : findPackageRoot(appName, options, appsDir)
  const root = packageRoot?.root ?? appsRoot
  const packageMetadata = packageRoot?.metadata ?? readPackageMetadata(root)
  const source = packageRoot ? 'package_root' : 'apps_dir'
  const useExplicit = options.explicitAppName === appName

  const readingsDir =
    useExplicit && options.explicitReadingsDir ? resolve(options.explicitReadingsDir) : join(root, 'readings')
  const dbPath = useExplicit && options.explicitDbPath ? resolve(options.explicitDbPath) : discoverDbPath(root, appName)

  const exists = isDirectory(root)
  const hasReadings = isDirectory(readingsDir)
  const hasDb = isFile(dbPath)
  const hasAppMarker = isFile(join(root, 'app.md')) || isFile(join(readingsDir, 'app.md'))

  return {
    name: appName,
    root,
    readingsDir,
    dbPath,
    exists,
    hasReadings,
    hasDb,
    hasAppMarker,
    source,
    ...(packageMetadata?.name ? { packageName: packageMetadata.name } : {}),
    ...(packageMetadata?.kind ? { packageKind: packageMetadata.kind } : {}),
    ...(packageMetadata?.description ? { packageDescription: packageMetadata.description } : {}),
    ...(packageMetadata?.dependencies ? { packageDependencies: packageMetadata.dependencies } : {}),
  }
}

export function listArestApps(options: AppRegistryOptions = {}): ArestApp[] {
  const appsDir = resolve(options.appsDir || defaultAppsDir(options.cwd))
  const apps = new Map<string, ArestApp>()

  if (isDirectory(appsDir)) {
    for (const entry of readdirSync(appsDir)) {
      if (!isDirectory(join(appsDir, entry))) continue
      try {
        const app = resolveArestApp(entry, options)
        apps.set(app.name, app)
      } catch {}
    }
  }

  for (const libraryRoot of discoverPackageRoots(options, appsDir)) {
    if (apps.has(libraryRoot.name)) continue
    apps.set(libraryRoot.name, resolveArestApp(libraryRoot.name, options))
  }

  return Array.from(apps.values()).sort((a, b) => a.name.localeCompare(b.name))
}

export function listArestReadingFiles(readingsDir: string): ArestAppFileInfo[] {
  return listReadingFiles(readingsDir)
}

export function createArestApp(name: string, options: AppRegistryOptions = {}, reading?: string): ArestApp {
  const app = resolveArestApp(name, options)
  mkdirSync(app.readingsDir, { recursive: true })
  if (reading && reading.trim()) {
    const readingPath = join(app.readingsDir, 'app.md')
    if (isFile(readingPath)) {
      throw new Error(`Initial reading already exists for AREST app: ${app.name}`)
    }
    writeFileSync(readingPath, reading, 'utf8')
  }
  return resolveArestApp(app.name, options)
}

export function managedInstanceReadingPath(readingsDir: string): string {
  return join(readingsDir, 'instances', 'mcp.md')
}

function validateFormlTerm(label: string, value: string, warnings: string[]): string | null {
  const term = value.trim()
  if (!term || !SAFE_FORML_TERM.test(term)) {
    warnings.push(`${label} is not safe to write as a FORML term: ${JSON.stringify(value)}`)
    return null
  }
  return term
}

function quoteFormlLiteral(label: string, value: string, warnings: string[]): string | null {
  if (value.includes("'") || /[\r\n]/.test(value)) {
    warnings.push(`${label} is not safe to write as a single-quoted FORML literal: ${JSON.stringify(value)}`)
    return null
  }
  return `'${value}'`
}

function resultEntityId(result: unknown, noun: string): string | null {
  if (!result || typeof result !== 'object') return null
  const entities = (result as { entities?: unknown }).entities
  if (!Array.isArray(entities)) return null
  const match = entities.find((entity) => {
    if (!entity || typeof entity !== 'object') return false
    return (entity as { type?: unknown }).type === noun && typeof (entity as { id?: unknown }).id === 'string'
  })
  return match ? (match as { id: string }).id : null
}

function instantiateFactTypeReading(
  factType: ManagedInstanceFactTypeReading,
  roleValues: string[],
  warnings: string[],
): string | null {
  if (factType.roles.length !== roleValues.length) {
    warnings.push(`fact type reading role count mismatch: ${factType.reading}`)
    return null
  }

  let cursor = 0
  let out = ''
  for (let i = 0; i < factType.roles.length; i += 1) {
    const role = factType.roles[i]
    const value = quoteFormlLiteral(role, roleValues[i], warnings)
    if (!value) return null
    const found = factType.reading.indexOf(role, cursor)
    if (found < 0) {
      warnings.push(`fact type reading does not contain role ${JSON.stringify(role)}: ${factType.reading}`)
      return null
    }
    out += factType.reading.slice(cursor, found)
    out += `${role} ${value}`
    cursor = found + role.length
  }
  out += factType.reading.slice(cursor)
  return `${out.trim()}.`
}

function exactInstanceFactLine(
  input: ApplyInstanceFactInput,
  rawField: string,
  rawValue: string,
  entityId: string,
  factType: ManagedInstanceFactTypeReading,
  warnings: string[],
): string | null {
  const roleValues: string[] = []
  for (let i = 0; i < factType.roles.length; i += 1) {
    const role = factType.roles[i]
    if (i === 0 && role === input.noun) {
      roleValues.push(entityId)
    } else if (role === rawField) {
      roleValues.push(rawValue)
    } else if (role === input.noun) {
      roleValues.push(entityId)
    } else {
      warnings.push(`no apply value for role ${JSON.stringify(role)} in ${factType.reading}`)
      return null
    }
  }
  return instantiateFactTypeReading(factType, roleValues, warnings)
}

export function buildApplyInstanceFacts(
  input: ApplyInstanceFactInput,
  result?: unknown
): ManagedInstanceFactBuildResult {
  const warnings: string[] = []
  if (input.operation !== 'create' && input.operation !== 'update') {
    return { lines: [], warnings }
  }

  const noun = validateFormlTerm('noun', input.noun, warnings)
  if (!noun) return { lines: [], warnings }

  const entityId = input.id ?? resultEntityId(result, noun)
  if (!entityId) {
    warnings.push(`instance facts for ${noun} require an id`)
    return { lines: [], warnings }
  }

  const quotedId = quoteFormlLiteral(`${noun} id`, entityId, warnings)
  if (!quotedId) return { lines: [], warnings }

  const lines: string[] = []
  const fields = input.fields ?? {}
  for (const [rawField, rawValue] of Object.entries(fields).sort(([left], [right]) => left.localeCompare(right))) {
    const factType = input.fieldFactTypes?.[rawField]
    if (factType) {
      const line = exactInstanceFactLine(input, rawField, rawValue, entityId, factType, warnings)
      if (line) lines.push(line)
      continue
    }

    const field = validateFormlTerm('field', rawField, warnings)
    const value = quoteFormlLiteral(`${noun}.${rawField}`, rawValue, warnings)
    if (!field || !value) continue
    lines.push(`${noun} ${quotedId} has ${field} ${value}.`)
  }
  if (lines.length === 0) {
    warnings.push(`no safe instance field facts were generated for ${noun} ${JSON.stringify(entityId)}`)
  }

  return { lines, warnings }
}

export function appendManagedInstanceFacts(
  readingsDir: string,
  lines: string[]
): ManagedInstanceFactAppendResult {
  const path = managedInstanceReadingPath(readingsDir)
  mkdirSync(dirname(path), { recursive: true })

  const existing = existsSync(path) ? readFileSync(path, 'utf8') : ''
  const existingLines = new Set(existing.split(/\r?\n/).map((line) => line.trim()).filter(Boolean))
  const newLines = lines.filter((line) => !existingLines.has(line.trim()))

  if (!existing.trim()) {
    writeFileSync(path, MANAGED_INSTANCE_READING_HEADER, 'utf8')
  }

  if (newLines.length > 0) {
    const current = readFileSync(path, 'utf8')
    const prefix = current.endsWith('\n') ? '' : '\n'
    appendFileSync(path, `${prefix}${newLines.join('\n')}\n`, 'utf8')
  }

  return {
    path,
    appended: newLines.length,
    skipped: lines.length - newLines.length,
    lines: newLines,
  }
}

export function inspectArestApp(app: ArestApp, options: AppRegistryOptions = {}): ArestAppInspection {
  const readingFiles = listReadingFiles(app.readingsDir)
  const newestReading = newestFile(readingFiles)
  const dependencies = dependencyClosure(app, options)
  const newestDependency = newestDependencyReading(dependencies.closure)
  const dbInfo = fileInfo(app.dbPath)
  const ownDbStale = Boolean(dbInfo && newestReading && dbInfo.modifiedMs < newestReading.modifiedMs)
  const dependencyDbStale = Boolean(dbInfo && newestDependency && dbInfo.modifiedMs < newestDependency.newestModifiedMs)
  const dbStale = ownDbStale || dependencyDbStale

  let status: ArestAppHealthStatus = 'ready'
  const issues: string[] = []
  const next_actions: ArestAppNextAction[] = []

  if (!app.exists) {
    status = 'not_found'
    issues.push('app root does not exist')
    next_actions.push({
      tool: 'apps.create',
      args: { name: app.name },
      reason: 'create the app root and readings directory',
    })
  } else if (app.packageKind === 'library' || (!app.hasDb && !app.hasAppMarker)) {
    status = 'library'
  } else if (!app.hasReadings || readingFiles.length === 0) {
    status = 'missing_readings'
    issues.push(app.hasReadings ? 'readings directory has no .md files' : 'readings directory is missing')
    next_actions.push({
      tool: 'apps.create',
      args: { name: app.name },
      reason: 'create or populate the readings directory',
    })
  } else if (!dbInfo) {
    status = 'needs_compile'
    issues.push('SQLite DB is missing')
    next_actions.push({
      tool: 'apps.compile',
      args: { name: app.name },
      reason: 'compile readings into the app SQLite DB',
    })
  } else if (dbStale) {
    status = 'stale_db'
    if (ownDbStale) issues.push('SQLite DB is older than one or more readings')
    if (dependencyDbStale) issues.push('SQLite DB is older than one or more dependency readings')
    next_actions.push({
      tool: 'apps.compile',
      args: { name: app.name },
      reason: dependencyDbStale
        ? 'refresh the SQLite DB from app readings and dependency readings'
        : 'refresh the SQLite DB from readings',
    })
  }

  return {
    ...app,
    health: {
      status,
      ok: status === 'ready' || status === 'library',
      issues,
      next_actions,
      readings: {
        dir: app.readingsDir,
        exists: app.hasReadings,
        count: readingFiles.length,
        newestModified: newestReading?.modified ?? null,
        newestModifiedMs: newestReading?.modifiedMs ?? null,
        files: readingFiles,
      },
      db: {
        path: app.dbPath,
        exists: Boolean(dbInfo),
        bytes: dbInfo?.bytes ?? null,
        modified: dbInfo?.modified ?? null,
        modifiedMs: dbInfo?.modifiedMs ?? null,
        stale: dbStale,
      },
      dependencies: {
        direct: dependencies.direct,
        closure: dependencies.closure,
        newestModified: newestDependency?.newestModified ?? null,
        newestModifiedMs: newestDependency?.newestModifiedMs ?? null,
        stale: dependencyDbStale,
      },
    },
  }
}

export function checkArestApps(options: AppRegistryOptions = {}): ArestAppsCheck {
  const apps = listArestApps(options).map((app) => inspectArestApp(app, options))
  const summary = {
    total: apps.length,
    ready: 0,
    library: 0,
    not_found: 0,
    missing_readings: 0,
    needs_compile: 0,
    stale_db: 0,
    issue_count: 0,
  }
  for (const app of apps) {
    summary[app.health.status] += 1
    if (!app.health.ok) summary.issue_count += 1
  }
  return { summary, apps }
}

function discoverDbPath(root: string, appName: string): string {
  const candidates = [join(root, `${appName}.db`), join(root, `${appName}-arest.db`), join(root, 'app.db'), join(root, 'arest.db')]
  const existingCandidate = candidates.find((candidate) => isFile(candidate))
  if (existingCandidate) return existingCandidate

  const dbs = listDbFiles(root)
  if (dbs.length === 1) return dbs[0]

  return candidates[0]
}

interface PackageMetadata {
  name?: string
  kind?: 'app' | 'library' | string
  description?: string
  keywords?: string[]
  dependencies?: Record<string, string>
}

interface DiscoveredPackageRoot {
  name: string
  root: string
  metadata: PackageMetadata
}

function findPackageRoot(name: string, options: AppRegistryOptions, appsDir: string): DiscoveredPackageRoot | null {
  return discoverPackageRoots(options, appsDir).find((root) => root.name === name) ?? null
}

function discoverPackageRoots(options: AppRegistryOptions = {}, appsDir?: string): DiscoveredPackageRoot[] {
  const roots: DiscoveredPackageRoot[] = []
  const appsDirPath = resolve(appsDir || options.appsDir || defaultAppsDir(options.cwd))
  for (const libraryDir of librarySearchDirs(options, appsDirPath)) {
    if (!isDirectory(libraryDir)) continue
    for (const entry of readdirSync(libraryDir)) {
      if (entry === 'node_modules' || entry.startsWith('.')) continue
      const root = join(libraryDir, entry)
      if (!isDirectory(root)) continue
      if (samePath(root, appsDirPath)) continue
      const metadata = readPackageMetadata(root)
      if (!isArestPackage(metadata)) continue
      try {
        roots.push({
          name: normalizeAppName(entry),
          root,
          metadata,
        })
      } catch {}
    }
  }
  return roots.sort((a, b) => a.name.localeCompare(b.name))
}

function librarySearchDirs(options: AppRegistryOptions, appsDir: string): string[] {
  const dirs = options.libraryDirs !== undefined
    ? options.libraryDirs
    : [dirname(appsDir)]
  return Array.from(new Set(dirs.map((dir) => resolve(dir))))
}

function readPackageMetadata(root: string): PackageMetadata | null {
  try {
    const raw = readFileSync(join(root, 'package.json'), 'utf8')
    const parsed = JSON.parse(raw) as PackageMetadata
    return parsed && typeof parsed === 'object' ? parsed : null
  } catch {
    return null
  }
}

function isArestPackage(metadata: PackageMetadata | null): metadata is PackageMetadata {
  if (!metadata) return false
  if (metadata.kind === 'app' || metadata.kind === 'library') return true
  return false
}

function dependencyClosure(app: ArestApp, options: AppRegistryOptions): {
  direct: ArestAppDependency[]
  closure: ArestAppDependency[]
} {
  const direct = directDependencies(app, options)
  const closure: ArestAppDependency[] = []
  const seen = new Set<string>()
  const visit = (dependency: ArestAppDependency) => {
    if (seen.has(dependency.name)) return
    seen.add(dependency.name)
    closure.push(dependency)
    if (!dependency.exists) return
    const dependencyApp = resolveArestApp(dependency.name, options)
    for (const nested of directDependencies(dependencyApp, options)) visit(nested)
  }
  for (const dependency of direct) visit(dependency)
  return { direct, closure }
}

function directDependencies(app: ArestApp, options: AppRegistryOptions): ArestAppDependency[] {
  const dependencies = app.packageDependencies || {}
  return Object.entries(dependencies)
    .map(([packageName, packageSpec]) => resolveDependency(app, packageName, packageSpec, options))
    .filter((dependency): dependency is ArestAppDependency => Boolean(dependency))
    .sort((a, b) => a.name.localeCompare(b.name))
}

function resolveDependency(
  app: ArestApp,
  packageName: string,
  packageSpec: string,
  options: AppRegistryOptions,
): ArestAppDependency | null {
  const root = dependencyRoot(app.root, packageName, packageSpec, options)
  if (!root) return null
  let name: string
  try {
    name = normalizeAppName(basename(root))
  } catch {
    return null
  }
  const dependencyApp = resolveArestApp(name, options)
  const files = listReadingFiles(dependencyApp.readingsDir)
  const newest = newestFile(files)
  return {
    name: dependencyApp.name,
    root: dependencyApp.root,
    readingsDir: dependencyApp.readingsDir,
    ...(dependencyApp.packageName ? { packageName: dependencyApp.packageName } : { packageName }),
    ...(dependencyApp.packageKind ? { packageKind: dependencyApp.packageKind } : {}),
    packageSpec,
    exists: dependencyApp.exists,
    hasReadings: dependencyApp.hasReadings,
    source: dependencyApp.source,
    readings: {
      count: files.length,
      newestModified: newest?.modified ?? null,
      newestModifiedMs: newest?.modifiedMs ?? null,
    },
  }
}

function dependencyRoot(
  appRoot: string,
  packageName: string,
  packageSpec: string,
  options: AppRegistryOptions,
): string | null {
  if (packageSpec.startsWith('file:')) return resolve(appRoot, packageSpec.slice('file:'.length))

  const appsDir = resolve(options.appsDir || defaultAppsDir(options.cwd))
  const packageRoot = discoverPackageRoots(options, appsDir)
    .find((root) => root.metadata.name === packageName)
  return packageRoot?.root ?? null
}

function newestDependencyReading(dependencies: ArestAppDependency[]): {
  newestModified: string
  newestModifiedMs: number
} | null {
  const candidates = dependencies
    .map((dependency) => dependency.readings.newestModified && dependency.readings.newestModifiedMs !== null
      ? {
          newestModified: dependency.readings.newestModified,
          newestModifiedMs: dependency.readings.newestModifiedMs,
        }
      : null)
    .filter((candidate): candidate is { newestModified: string; newestModifiedMs: number } => Boolean(candidate))
  if (!candidates.length) return null
  return candidates.reduce((latest, candidate) =>
    candidate.newestModifiedMs > latest.newestModifiedMs ? candidate : latest,
  )
}

function listDbFiles(root: string): string[] {
  if (!isDirectory(root)) return []
  return readdirSync(root)
    .filter((entry) => entry.toLowerCase().endsWith('.db'))
    .map((entry) => join(root, entry))
    .filter(isFile)
}

function listReadingFiles(readingsDir: string): ArestAppFileInfo[] {
  if (!isDirectory(readingsDir)) return []
  const files: ArestAppFileInfo[] = []
  collectReadingFiles(readingsDir, readingsDir, files)
  return files.sort((a, b) => a.path.localeCompare(b.path))
}

function collectReadingFiles(root: string, dir: string, out: ArestAppFileInfo[]): void {
  for (const entry of readdirSync(dir)) {
    if (entry.startsWith('.')) continue
    const path = join(dir, entry)
    let stats
    try {
      stats = statSync(path)
    } catch {
      continue
    }
    if (stats.isDirectory()) {
      collectReadingFiles(root, path, out)
    } else if (stats.isFile() && entry.toLowerCase().endsWith('.md')) {
      out.push({
        name: relative(root, path) || basename(path),
        path,
        bytes: stats.size,
        modifiedMs: stats.mtimeMs,
        modified: stats.mtime.toISOString(),
      })
    }
  }
}

function newestFile(files: ArestAppFileInfo[]): ArestAppFileInfo | null {
  if (!files.length) return null
  return files.reduce((latest, file) => file.modifiedMs > latest.modifiedMs ? file : latest)
}

function fileInfo(path: string): ArestAppFileInfo | null {
  try {
    const stats = statSync(path)
    if (!stats.isFile()) return null
    return {
      name: basename(path),
      path,
      bytes: stats.size,
      modifiedMs: stats.mtimeMs,
      modified: stats.mtime.toISOString(),
    }
  } catch {
    return null
  }
}

function isDirectory(path: string): boolean {
  try {
    return statSync(path).isDirectory()
  } catch {
    return false
  }
}

function isFile(path: string): boolean {
  try {
    return existsSync(path) && statSync(path).isFile()
  } catch {
    return false
  }
}

function samePath(a: string, b: string): boolean {
  return resolve(a).toLowerCase() === resolve(b).toLowerCase()
}
