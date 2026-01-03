import { createReadStream, createWriteStream, readdirSync, statSync, mkdirSync, existsSync } from 'fs'
import { createGzip } from 'zlib'
import { join, dirname } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const distDir = join(__dirname, '..', 'dist')
const outputDir = join(__dirname, '..', '..', '..', '..', 'data')

// Ensure output directory exists
if (!existsSync(outputDir)) {
  mkdirSync(outputDir, { recursive: true })
}

function gzipFile(src, dest) {
  return new Promise((resolve, reject) => {
    const readStream = createReadStream(src)
    const writeStream = createWriteStream(dest)
    const gzip = createGzip({ level: 9 })

    readStream
      .pipe(gzip)
      .pipe(writeStream)
      .on('finish', () => {
        const srcSize = statSync(src).size
        const destSize = statSync(dest).size
        const ratio = ((1 - destSize / srcSize) * 100).toFixed(1)
        console.log(`  ${src.split('/').pop()} → ${dest.split('/').pop()} (${ratio}% smaller)`)
        resolve()
      })
      .on('error', reject)
  })
}

async function main() {
  console.log('Gzipping build output for ESP32...\n')

  const files = readdirSync(distDir)

  for (const file of files) {
    const srcPath = join(distDir, file)
    const stat = statSync(srcPath)

    if (stat.isFile() && !file.endsWith('.gz')) {
      const destPath = join(outputDir, file + '.gz')
      await gzipFile(srcPath, destPath)
    }
  }

  console.log('\n✓ Done! Gzipped files copied to data/ directory')
}

main().catch(console.error)
