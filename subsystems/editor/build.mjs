// Build-time only: all runtime assets are served locally by the Rust operator.
import { build } from 'esbuild';
import { mkdir, readdir, readFile, writeFile, copyFile } from 'node:fs/promises';
import { resolve, join, relative } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('.', import.meta.url));
const out = resolve(process.argv[2] || join(root, 'dist'));
const distribution = JSON.parse(await readFile(join(root, 'node_modules/monaco-editor/package.json'), 'utf8'));
const declared = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
if (distribution.version !== declared.dependencies['monaco-editor']) throw Error('Monaco dependency identity differs from pinned package');
await mkdir(out, { recursive: false });
await build({ entryPoints: { editor: join(root, 'src/editor.js'), 'editor.worker': join(root, 'node_modules/monaco-editor/esm/vs/editor/editor.worker.js') }, outdir: out, bundle: true, splitting: true, format: 'esm', target: 'es2022', minify: true, sourcemap: false, loader: { '.ttf': 'file' }, assetNames: 'assets/[name]-[hash]', chunkNames: 'chunks/[name]-[hash]', legalComments: 'external' });
await copyFile(join(root, 'node_modules/monaco-editor/LICENSE'), join(out, 'MONACO-LICENSE.txt'));
await copyFile(join(root, 'node_modules/monaco-editor/ThirdPartyNotices.txt'), join(out, 'MONACO-ThirdPartyNotices.txt'));
const files = {};
async function walk(dir) { for (const entry of await readdir(dir, { withFileTypes: true })) { const file = join(dir, entry.name); if (entry.isDirectory()) await walk(file); else { const bytes = await readFile(file); files[relative(out, file).replaceAll('\\', '/')] = { sha256: createHash('sha256').update(bytes).digest('hex'), size_bytes: bytes.length }; } } }
await walk(out);
const manifest = { schema_version: 1, monaco_version: distribution.version, entry: 'editor.js', files };
const manifestBytes = JSON.stringify(manifest, null, 2) + '\n';
await writeFile(join(out, 'manifest.json'), manifestBytes);
console.log(JSON.stringify({ directory: out, files: Object.keys(files).length, bytes: Object.values(files).reduce((n, f) => n + f.size_bytes, 0), manifest_sha256: createHash('sha256').update(manifestBytes).digest('hex'), model_calls: false }));
