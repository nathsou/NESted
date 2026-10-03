import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';
const root = resolve(import.meta.dirname, '..');
const env = { ...process.env };
if (existsSync('/workspace/.cargo/bin/cargo')) {
  env.CARGO_HOME = '/workspace/.cargo'; env.RUSTUP_HOME = '/workspace/.rustup'; env.PATH = `/workspace/.cargo/bin:${env.PATH}`;
}
function run(command,args) { const result=spawnSync(command,args,{cwd:root,env,stdio:'inherit'});if(result.status!==0)process.exit(result.status??1); }
run('python',['scripts/generate-assets.py']);
run('cargo',['build','--release','--locked','-p','nested-wasm','--target','wasm32-unknown-unknown']);
// The CLI has a native build for VS Code and game verification.
run('cargo',['build','--release','--locked','-p','nested-cli']);
mkdirSync(resolve(root,'playground/public/examples'),{recursive:true});
mkdirSync(resolve(root,'playground/public/assets'),{recursive:true});
copyFileSync(resolve(root,'target/wasm32-unknown-unknown/release/nested_wasm.wasm'),resolve(root,'playground/public/nested.wasm'));
for(const file of readdirSync(resolve(root,'games')))if(file.endsWith('.nst'))copyFileSync(resolve(root,'games',file),resolve(root,'playground/public/examples',file));
for(const file of readdirSync(resolve(root,'games/assets')))copyFileSync(resolve(root,'games/assets',file),resolve(root,'playground/public/assets',file));
run('npm',['run','typecheck']);
run('npm',['run','build:web']);
