// One-off SVG → PNG converter for tray icons (32x32).
// Run: node tools/svg-to-png.cjs
const fs = require('node:fs')
const path = require('node:path')
const { Resvg } = require('@resvg/resvg-js')

const ROOT = path.resolve(__dirname, '..')
const targets = [
  { svg: 'src-tauri/icons/tray-active.svg', png: 'src-tauri/icons/tray-active.png', size: 32 },
  { svg: 'src-tauri/icons/tray-idle.svg',   png: 'src-tauri/icons/tray-idle.png',   size: 32 },
  // Also generate @2x for high-DPI tray rendering (64x64)
  { svg: 'src-tauri/icons/tray-active.svg', png: 'src-tauri/icons/tray-active@2x.png', size: 64 },
  { svg: 'src-tauri/icons/tray-idle.svg',   png: 'src-tauri/icons/tray-idle@2x.png',   size: 64 },
]

for (const t of targets) {
  const svg = fs.readFileSync(path.join(ROOT, t.svg), 'utf8')
  const r = new Resvg(svg, {
    fitTo: { mode: 'width', value: t.size },
    background: 'rgba(0,0,0,0)',
  })
  const png = r.render().asPng()
  const out = path.join(ROOT, t.png)
  fs.writeFileSync(out, png)
  console.log(`OK  ${t.svg}  ->  ${t.png}  (${png.length} bytes)`)
}
console.log('done')
