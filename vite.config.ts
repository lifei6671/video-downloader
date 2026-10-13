import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';
import tailwindcss from '@tailwindcss/vite';
const packageVersion = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version;
const version = (process.env.APP_VERSION || (process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : '') || packageVersion).replace(/^v/, '');
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error('Build version must be a version such as 1.2.3 or v1.2.3-beta.1');
}
export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(version) },
  plugins: [tailwindcss()],
  resolve: { alias: { '@': new URL('./src', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1') } },
  server: {
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/python-engine/**', '**/.venv/**', '**/.local/**'] },
  },
});
