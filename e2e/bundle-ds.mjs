import { build } from './node_modules/esbuild/lib/main.js';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const base = path.join(__dirname, 'node_modules', '@starfederation', 'datastar', 'dist');

function resolveWithExt(p) {
  if (fs.existsSync(p) && fs.statSync(p).isFile()) return p;
  if (fs.existsSync(p + '.js')) return p + '.js';
  if (fs.existsSync(p + '/index.js')) return p + '/index.js';
  return null;
}

await build({
  entryPoints: ['ds-entry.mjs'],
  bundle: true,
  format: 'esm',
  minify: true,
  outfile: '../web/static/datastar.js',
  plugins: [{
    name: 'ds-resolver',
    setup(build) {
      build.onResolve({ filter: /^~\// }, (args) => {
        const rel = args.path.replace(/^~\//, '');
        const full = resolveWithExt(path.resolve(base, rel));
        return { path: full, errors: full ? [] : [{ text: `unresolved ~ path: ${args.path}` }] };
      });
      build.onResolve({ filter: /^\./ }, (args) => {
        const full = resolveWithExt(path.resolve(args.resolveDir, args.path));
        return { path: full, errors: full ? [] : [{ text: `unresolved rel path: ${args.path} in ${args.resolveDir}` }] };
      });
    },
  }],
  logLevel: 'silent',
});

console.log('bundle done');
