// Writes the app icon source (app/brand/icon.svg) from the mark's own geometry, so the icon
// and the UI never drift apart. Then `npx tauri icon brand/icon.svg` (from app/)
// makes every size. Run: node --experimental-strip-types app/brand/icon.ts
import { writeFileSync } from 'node:fs';
import { LINES, MARK_STROKE, ribbonPath } from '../src/lib/mark.ts';

// The accent tile with a little light at the top, and the mark in the accent's ink.
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="1024" height="1024">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#7d8cff"/>
      <stop offset="1" stop-color="#5d6cf2"/>
    </linearGradient>
  </defs>
  <rect x="1" y="1" width="62" height="62" rx="14.5" fill="url(#tile)"/>
  <g fill="none" stroke="#0e1330" stroke-width="${MARK_STROKE}" stroke-linecap="round" stroke-linejoin="round">
    <path d="${ribbonPath()}"/>
${LINES.map((d) => `    <path d="${d}"/>`).join('\n')}
  </g>
</svg>
`;
writeFileSync(new URL('./icon.svg', import.meta.url), svg);
console.log('wrote app/brand/icon.svg');
