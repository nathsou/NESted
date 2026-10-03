import assert from 'node:assert/strict';
import {readFileSync,readdirSync,mkdirSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {resolve} from 'node:path';
import {Wasm} from '../playground/src/wasm.ts';
const root=resolve(import.meta.dirname,'..');
const module=await WebAssembly.compile(readFileSync(resolve(root,'playground/public/nested.wasm')));
const wasm=new Wasm(await WebAssembly.instantiate(module,{}));
for(const name of readdirSync(resolve(root,'games/assets')))wasm.asset(`assets/${name}`,readFileSync(resolve(root,'games/assets',name)));
const e=wasm.exports;
const results=[];
for(const game of ['bloom','starstring','skythread','emberkeep']){
 const source=readFileSync(resolve(root,`games/${game}.nst`),'utf8');const start=performance.now();
 assert.equal(wasm.input(source,(p,n)=>e.nested_compile(p,n,1)),1,JSON.stringify(wasm.result()));const build=wasm.result(),compileMs=performance.now()-start;
 const rom=new Uint8Array(e.memory.buffer,e.nested_rom_ptr(),e.nested_rom_len()).slice();assert.equal(rom[6]>>4,({bloom:0,starstring:4,skythread:1,emberkeep:2})[game]);
 mkdirSync(resolve(root,'artifacts'),{recursive:true});const native=spawnSync(resolve(root,'target/release/nested'),['build',`games/${game}.nst`,'-o',`artifacts/${game}.nes`],{cwd:root,encoding:'utf8'});assert.equal(native.status,0,native.stderr);assert.deepEqual(Buffer.from(rom),readFileSync(resolve(root,`artifacts/${game}.nes`)),'Native and WASM ROMs must match byte for byte');
 assert.equal(wasm.input(rom,(p,n)=>e.nested_load_rom(p,n,48000)),1);
 let squares=0,samples=0;const runStart=performance.now();for(let frame=0;frame<180;frame++){assert.equal(e.nested_frame(0),1);for(const value of new Float32Array(e.memory.buffer,e.nested_audio_ptr(),e.nested_audio_len())){assert.ok(Number.isFinite(value));squares+=value*value;samples++;}}
 const fps=180000/(performance.now()-runStart);assert.ok(Math.sqrt(squares/samples)>0.001,`${game} must have real APU music`);assert.equal(new Uint8Array(e.memory.buffer,e.nested_ram_ptr(),2048)[0x37f],0,'VRAM queue must not overflow');assert.equal(build.passes.length,7);results.push({game,code:build.codeBytes,ram:build.ramBytes,compileMs:Math.round(compileMs),emulatorFps:Math.round(fps),rms:Math.sqrt(squares/samples).toFixed(4)});
}
console.table(results);
const uri='file:///typing.nst';let sequence=1;const rpc=(method,params)=>wasm.rpc({jsonrpc:'2.0',id:sequence++,method,params});
assert.equal(rpc('initialize',{})[0].result.capabilities.positionEncoding,'utf-16');
const source='var score:u8=0; fn update(){ score += 1; }';rpc('textDocument/didOpen',{textDocument:{uri,languageId:'nested',version:1,text:source}});
assert.ok(rpc('textDocument/completion',{textDocument:{uri},position:{line:0,character:30}})[0].result.items.some(i=>i.label==='score'));
assert.equal(rpc('textDocument/rename',{textDocument:{uri},position:{line:0,character:5},newName:'points'})[0].result.changes[uri].length,2);
rpc('textDocument/didChange',{textDocument:{uri,version:2},contentChanges:[{text:'var score:u8=0; fn update(){ score += '} ]});
assert.ok(rpc('textDocument/completion',{textDocument:{uri},position:{line:0,character:37}})[0].result.items.some(i=>i.label==='score'),'Completion survives unfinished statements');
const {createRequire}=await import('node:module');const require=createRequire(import.meta.url);const {LspTransport}=require('../extension/transport.cjs');const transport=new LspTransport(resolve(root,'target/release/nested'));
try{const init=await transport.request('initialize',{});assert.equal(init.serverInfo.name,'NESted Language Server');transport.notify('initialized',{});transport.notify('textDocument/didOpen',{textDocument:{uri,languageId:'nested',version:1,text:source}});const hover=await transport.request('textDocument/hover',{textDocument:{uri},position:{line:0,character:5}});assert.match(hover.contents.value,/score/);const tokens=await transport.request('textDocument/semanticTokens/full',{textDocument:{uri}});assert.ok(tokens.data.length>20);}finally{await transport.close();}
console.log('Native/WASM parity, APU audio, four ROMs, shared LSP, and VS Code transport passed.');
