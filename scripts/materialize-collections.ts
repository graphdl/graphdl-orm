import { getPayload } from 'payload'
import fs from 'fs'
import path from 'path'

const GENERATED_DIR = path.resolve(__dirname, '../src/collections/generated')

async function materialize() {
  const { default: configPromise } = await import('../src/payload.config')
  const payload = await getPayload({ config: configPromise })

  // Find all generators with outputFormat = 'payload'
  const generators = await payload.find({
    collection: 'generators',
    where: { outputFormat: { equals: 'payload' } },
    pagination: false,
  })

  // Ensure output directory exists
  if (!fs.existsSync(GENERATED_DIR)) {
    fs.mkdirSync(GENERATED_DIR, { recursive: true })
  }

  // Clear existing generated files
  for (const file of fs.readdirSync(GENERATED_DIR)) {
    if (file.endsWith('.ts') && file !== 'index.ts') {
      fs.unlinkSync(path.join(GENERATED_DIR, file))
    }
  }

  // Write each generated collection file
  const slugs: string[] = []
  for (const gen of generators.docs) {
    const files = (gen as any).output?.files || {}
    for (const [filePath, content] of Object.entries(files)) {
      const outPath = path.join(GENERATED_DIR, path.basename(filePath))
      fs.writeFileSync(outPath, content as string)
      const slug = path.basename(filePath, '.ts')
      slugs.push(slug)
    }
  }

  // Write barrel file
  const barrel = slugs.map(slug => {
    const pascalName = slug.split('-').map(s => s.charAt(0).toUpperCase() + s.slice(1)).join('')
    return `export { ${pascalName} } from './${slug}'`
  }).join('\n') + '\n'
  fs.writeFileSync(path.join(GENERATED_DIR, 'index.ts'), barrel)

  console.log(`Materialized ${slugs.length} collections to ${GENERATED_DIR}`)
  process.exit(0)
}

materialize().catch(console.error)
