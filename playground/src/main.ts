import './style.css';
import type { Build, Diagnostic, Position, Range, Rpc } from './wasm';
const icons = {
  play: '<svg viewBox="0 0 16 16"><path d="M4 2l10 6-10 6z"/></svg>',
  pause: '<svg viewBox="0 0 16 16"><path d="M3 2h4v12H3zm6 0h4v12H9z"/></svg>',
  reset: '<svg viewBox="0 0 16 16"><path d="M3 3v4h4M3 6a5 5 0 1 1 0 5" fill="none" stroke="currentColor" stroke-width="1.7"/></svg>',
  arrow: '<svg viewBox="0 0 16 16"><path d="M3 8h10M9 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.7"/></svg>',
  download: '<svg viewBox="0 0 16 16"><path d="M8 2v8m-3-3l3 3 3-3M3 11v3h10v-3" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
};
const games = [
  { id: 'bloom', name: 'Nonogram', genre: 'NONOGRAM', color: '#b8c5db', symbol: '▦', description: 'Sixteen logic puzzles. Fill the grid using the clues.', controls: 'Arrows move · Z fills · X marks · Shift undoes · Enter next puzzle', song: 'Quiet Patterns', mapper: 'NROM' },
  { id: 'starstring', name: 'Starstring', genre: 'RHYTHM', color: '#b5a3f8', symbol: '♫', description: 'Chase the melody through a neon night.', controls: 'Arrows hit / hold · Shift difficulty · Enter starts / pauses / retries', song: 'Moonlight Circuit', mapper: 'MMC3' },
  { id: 'skythread', name: 'Skythread', genre: 'PLATFORMER', color: '#8dcbef', symbol: '▲', description: 'A small climber. An impossible-looking ascent.', controls: 'Arrows move · Z jumps · X dashes · Enter restarts', song: 'Above the Clouds', mapper: 'MMC1' },
  { id: 'emberkeep', name: 'Emberkeep', genre: 'ROGUELIKE', color: '#f2bb76', symbol: '◆', description: 'Keep your lantern alive. Find the stairs.', controls: 'Arrows move / attack · Z waits · X potion · Enter pauses / new run', song: 'The Last Lantern', mapper: 'UxROM' },
];
const app = document.querySelector<HTMLDivElement>('#app')!;
app.innerHTML = `
<header class="header"><a class="brand" href="./" aria-label="NESted home"><span class="brand-mark">N</span><strong>NESted</strong></a><select id="game-select" aria-label="Cartridge">${games.map(g=>`<option value="${g.id}">${g.name} · ${g.mapper}</option>`).join('')}</select><button id="build" class="primary" title="Build & run (Ctrl/Cmd+Enter)">${icons.play} Build</button><label class="optimize" title="Optimize generated machine code"><input id="optimize" type="checkbox" checked/><span>Optimize</span></label><nav><select id="layout-select" aria-label="Workspace layout"><option value="workbench">Workbench</option><option value="play">Play</option><option value="code">Code & output</option></select><a href="https://github.com/nathsou/NESted/blob/main/docs/language.md" target="_blank" rel="noreferrer">Guide</a><a href="https://github.com/nathsou/NESted/tree/main/extension" target="_blank" rel="noreferrer">VS Code</a><a href="https://github.com/nathsou/NESted" target="_blank" rel="noreferrer">GitHub</a></nav></header>
<nav class="mobile-tabs" aria-label="Workspace pane"><button data-pane="editor" class="active" aria-pressed="true">Source</button><button data-pane="player" aria-pressed="false">Game</button><button data-pane="inspector" aria-pressed="false">Output</button></nav>
<main class="workspace">
<section class="panel editor-panel"><div class="panel-toolbar"><div class="file-label"><span class="file-icon">N</span><span id="filename">bloom.nst</span><span id="dirty" title="Source changed since the last build"></span></div><div class="toolbar-actions"><button id="reset-source" title="Restore the bundled example; your previous draft can be restored">Reset example</button><label class="asset-upload">Open source<input id="source-file" type="file" accept=".nst,text/plain" hidden/></label></div></div><div class="editor-wrap"><div id="line-numbers" class="line-numbers" aria-hidden="true"></div><div class="code-area"><pre id="highlight" aria-hidden="true"></pre><textarea id="source" spellcheck="false" autocomplete="off" autocapitalize="off" aria-label="NESted source editor" wrap="off"></textarea></div><div id="completion" class="completion" hidden></div><div id="hover" class="hover-card" hidden></div></div><div class="editor-footer"><span id="cursor">Ln 1, Col 1</span><span>LSP <i id="lsp-led"></i></span><button id="format" title="Format (Shift+Alt+F)">Format</button><button id="save-source" title="Save .nst source">Save source</button></div><div class="console"><div class="console-title"><span>BUILD OUTPUT</span><span id="build-time"></span></div><div id="messages" role="status"><span class="muted">Loading the Rust/WASM compiler…</span></div></div></section>
<div class="pane-divider" data-divider="0" role="separator" tabindex="0" aria-label="Resize source pane" aria-orientation="vertical"></div>
<section class="panel player-panel"><div class="panel-toolbar"><div class="panel-heading"><span class="small-dot"></span> GAME</div><div class="toolbar-actions"><button id="pause" class="icon-button" title="Pause / play">${icons.pause}</button><button id="step" class="icon-button" title="Advance one emulated frame">${icons.arrow}</button><button id="reset" class="icon-button" title="Reset cartridge">${icons.reset}</button></div></div><div class="player-content"><div class="cartridge-meta"><span id="mapper-badge">NROM · NES</span><span><i class="running-dot"></i><span id="run-status">INITIALIZING</span></span></div><div class="screen-shell"><canvas id="screen" width="256" height="240" tabindex="0" aria-label="NES game screen. Click to play with the keyboard."></canvas><div id="screen-loading">Loading cartridge…</div></div><div class="play-hint">Click the screen for keyboard controls <span>${icons.arrow}</span></div><div class="game-caption"><span id="genre-label">NONOGRAM</span><h1 id="game-title">Nonogram</h1><p id="game-description">Sixteen logic puzzles.</p></div><div class="controller"><div class="dpad"><button data-button="16" class="up" aria-label="Up">▲</button><button data-button="64" class="left" aria-label="Left"><svg viewBox="0 0 12 12"><path d="M9 1L2 6l7 5z"/></svg></button><span class="middle"></span><button data-button="128" class="right" aria-label="Right"><svg viewBox="0 0 12 12"><path d="M3 1l7 5-7 5z"/></svg></button><button data-button="32" class="down" aria-label="Down">▼</button></div><div class="controller-middle"><button data-button="4">SELECT</button><button data-button="8">START</button></div><div class="ab-buttons"><button data-button="2"><span>X</span>B</button><button data-button="1"><span>Z</span>A</button></div></div><p id="controls" class="controls">Arrows move · Z fills · X marks · Shift undoes · Enter next puzzle</p><div class="audio-strip"><button id="sound" title="Enable original NES APU audio">♫ <span>Enable sound</span></button><span id="song-name">Quiet Patterns</span><span class="audio-bars"><i></i><i></i><i></i><i></i><i></i></span></div><div class="player-links"><button id="download-rom">${icons.download} Download .nes</button><label class="asset-upload">Add assets<input id="assets" type="file" multiple hidden/></label></div></div><div class="emulator-footer"><span id="fps">— fps</span><span id="pc">PC $0000</span><span>NESSY CORE</span></div></section>
<div class="pane-divider" data-divider="1" role="separator" tabindex="0" aria-label="Resize game pane" aria-orientation="vertical"></div>
<section class="panel inspector-panel"><div class="panel-toolbar inspector-tabs"><button class="active" data-inspector="assembly">Assembly</button><button data-inspector="passes">Passes</button><button data-inspector="memory">Memory</button></div><div class="inspector-subhead"><span id="inspector-title">6502 · FINAL MACHINE CODE</span><select id="pass-select" aria-label="Compiler pass" hidden></select><span id="instruction-cost">cycles</span></div><div id="inspector-code" class="inspector-code" tabindex="0"></div><div id="memory-view" class="memory-view" hidden></div><div class="inspector-footer"><span id="code-size">— bytes</span><span id="inspect-note">Cycle ranges exclude DMA & interrupts</span></div></section>
</main>
<footer class="statusbar"><div><span class="status-dot"></span><span id="status">Starting the workbench</span></div><div><span id="ram-status">RAM —</span><span id="zp-status">ZERO PAGE —</span><span id="rom-status">ROM —</span><span class="target-label">RICOH 2A03 / NTSC</span></div></footer>
<dialog id="rename-dialog"><form method="dialog"><h2>Rename symbol</h2><input id="rename-input" aria-label="New symbol name"/><div><button value="cancel">Cancel</button><button class="primary" id="rename-submit" value="rename">Rename</button></div></form></dialog>
`;
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const editor = $<HTMLTextAreaElement>('source');
const highlight = $('highlight');
const canvas = $<HTMLCanvasElement>('screen');
const ctx = canvas.getContext('2d', { alpha: false })!;
const image = ctx.createImageData(256, 240);
const compiler = new Worker(new URL('./compiler.worker.ts', import.meta.url), { type: 'module' });
const emulator = new Worker(new URL('./emulator.worker.ts', import.meta.url), { type: 'module' });
let gameIndex = 0;
let sourceVersion = 0;
let builtVersion = -1;
let build: Build | null = null;
let rom: ArrayBuffer | null = null;
let ram = new Uint8Array(2048);
let rpcId = 1;
let assetId = 10000;
let buildId = 20000;
let busy = false;
let framePending = false;
let playing = true;
let cartridgeLoaded = false;
let buttons = 0;
let frames = 0;
let fpsFrames = 0;
let fpsStart = performance.now();
let lastFrame = 0;
let debounce: ReturnType<typeof setTimeout>;
let diagnostics: Diagnostic[] = [];
let inspector = 'assembly';
let passIndex = 6;
let sound = false;
let audioContext: AudioContext | null = null;
let audioNode: AudioWorkletNode | null = null;
let currentUri = '';
let originalSource = '';
let previousDraft: string | null = null;
let emulationVersion = 0;
let loadingGame = false;
const requests = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
const assetRequests = new Map<number, () => void>();
let compilePending: Promise<Build> | null = null;
let compileResolver: ((report: Build) => void) | null = null;
let frameResolver: (() => void) | null = null;
function escape(s: string): string { return s.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;'); }
function pos(offset: number): Position { const s=editor.value.slice(0,offset).split('\n');return {line:s.length-1,character:s.at(-1)!.length}; }
function off(p: Position): number { const lines=editor.value.split('\n');let n=0;for(let i=0;i<Math.min(p.line,lines.length);i++)n+=lines[i].length+1;return n+p.character; }
function updateCursor(): void { const p=pos(editor.selectionStart);$('cursor').textContent=`Ln ${p.line+1}, Col ${p.character+1}`; }
function plainHighlight(): void {
  const text=editor.value;
  const regex=/(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')|\b(fn|var|let|const|if|else|for|in|while|loop|return|break|continue|asm|raw|match|cartridge|true|false)\b|\b(u8|i8|u16|i16|bool|void)\b|\b(0x[\da-fA-F_]+|0b[01_]+|\d[\d_]*)\b|(@\w+)/g;
  let html='';let end=0;for(const m of text.matchAll(regex)){html+=escape(text.slice(end,m.index));const type=m[1]?'comment':m[2]?'string':m[3]?'keyword':m[4]?'type':m[5]?'number':'attribute';html+=`<span class="tok-${type}">${escape(m[0])}</span>`;end=m.index!+m[0].length;}highlight.innerHTML=html+escape(text.slice(end))+'\n';
  $('line-numbers').innerHTML=Array.from({length:text.split('\n').length},(_,i)=>`<span>${i+1}</span>`).join('');
  syncScroll();
}
function syncScroll():void { highlight.scrollTop=editor.scrollTop;highlight.scrollLeft=editor.scrollLeft;$('line-numbers').scrollTop=editor.scrollTop; }
function syncDocument():void{clearTimeout(debounce);if(currentUri)compiler.postMessage({kind:'rpc',body:{jsonrpc:'2.0',method:'textDocument/didChange',params:{textDocument:{uri:currentUri,version:sourceVersion},contentChanges:[{text:editor.value}]}}});}
function rpc(method:string,params:unknown,notification=false):Promise<unknown>{
  if(method.startsWith('textDocument/')&&!['textDocument/didOpen','textDocument/didClose','textDocument/didChange'].includes(method))syncDocument();
  if(notification){compiler.postMessage({kind:'rpc',body:{jsonrpc:'2.0',method,params}});return Promise.resolve(null);}
  const id=rpcId++;return new Promise((resolve,reject)=>{requests.set(id,{resolve,reject});compiler.postMessage({kind:'rpc',body:{jsonrpc:'2.0',id,method,params}});});
}
function documentParams(extra:Record<string,unknown>={}):Record<string,unknown>{return{textDocument:{uri:currentUri},...extra};}
function changed():void {
  sourceVersion++;$('dirty').textContent='●';plainHighlight();updateCursor();
  try{localStorage.setItem(`nested-source-${games[gameIndex].id}`,editor.value);}catch{/* Storage may be disabled. */}
  clearTimeout(debounce);debounce=setTimeout(()=>{void rpc('textDocument/didChange',{textDocument:{uri:currentUri,version:sourceVersion},contentChanges:[{text:editor.value}]},true);},100);
}
function messages(values:Diagnostic[],summary?:string):void {
  const box=$('messages');box.replaceChildren();
  if(summary){const line=document.createElement('div');line.className='build-success';line.textContent=summary;box.append(line);}
  for(const d of values.slice(0,8)){const button=document.createElement('button');button.className='diagnostic';button.textContent=`${d.range.start.line+1}:${d.range.start.character+1}  ${d.message}`;button.onclick=()=>goto(d.range);box.append(button);}
  if(values.length===0&&!summary)box.innerHTML='<span class="muted">No diagnostics. The language server is ready.</span>';
}
function goto(range:Range):void { editor.focus();editor.setSelectionRange(off(range.start),off(range.end));editor.scrollTop=Math.max(0,(range.start.line-8)*21);syncScroll();updateCursor(); }
function renderInspector():void {
  if(!build?.ok)return;
  const view=$('inspector-code');const memory=$('memory-view');view.hidden=inspector==='memory';memory.hidden=inspector!=='memory';$<HTMLSelectElement>('pass-select').hidden=inspector!=='passes';$('instruction-cost').hidden=inspector!=='assembly';
  if(inspector==='memory'){ $('inspector-title').textContent='CPU RAM · LIVE VALUES';renderMemory(); }
  else {const selected=inspector==='assembly'?build.passes.at(-1)!:build.passes[passIndex];$('inspector-title').textContent=inspector==='assembly'?'6502 · FINAL MACHINE CODE':'PIPELINE · INSPECT EVERY STAGE';view.innerHTML=selected.text.split('\n').map(line=>{
    if(inspector==='assembly')return `<div>${escape(line).replace(/^([A-F\d]{4})/, '<span class="asm-address">$1</span>').replace(/(;.*)$/, '<span class="asm-comment">$1</span>').replace(/\b(lda|ldx|ldy|sta|stx|sty|adc|sbc|and|ora|eor|asl|lsr|rol|ror|cmp|cpx|cpy|beq|bne|bcc|bcs|bmi|bpl|jmp|jsr|rts|rti|clc|sec|pha|pla|tax|tay|txa|tya|inc|dec|inx|dex|iny|dey)\b/g,'<span class="asm-op">$1</span>')}</div>`;
    return `<div>${escape(line)||' '}</div>`;
  }).join('');}
  $('code-size').textContent=`${build.codeBytes.toLocaleString()} bytes of fixed code/data`;
  $('inspect-note').textContent=inspector==='assembly'?'Cycle ranges exclude DMA & interrupts':inspector==='memory'?'Static frames share storage':'Snapshots from this exact build';
}
function renderMemory():void {
  if(!build?.ok)return;const data=build.memory.filter(m=>m.kind!=='static frame (overlaid)');
  $('memory-view').innerHTML=`<div class="memory-summary"><strong>${build.ramBytes}<small>B</small></strong><span>COMPILER STORAGE<br/><b>${build.frameBytes} B</b> shared function frames</span></div><div class="memory-columns"><span>SYMBOL</span><span>ADDRESS</span><span>VALUE</span></div>`+data.map(m=>`<div class="memory-row"><span title="${escape(m.name)}">${escape(m.name)}</span><code>$${m.address.toString(16).toUpperCase().padStart(4,'0')}</code><b>${m.size<=2?ram[m.address]+(m.size===2?ram[m.address+1]*256:0):`[${m.size}]`}</b></div>`).join('')+`<div class="memory-note">${build.zeroPageBytes} B allocated in zero page.<br/>OAM, runtime scratch, VRAM queue, and the hardware stack have separate reservations.</div>`;
}
async function compileSource():Promise<Build>{
  if(busy&&compilePending)return compilePending;busy=true;$<HTMLSelectElement>('game-select').disabled=true;const button=$<HTMLButtonElement>('build');button.disabled=true;button.innerHTML='<span class="spinner"></span> Building';$('status').textContent='Compiling source to an NES cartridge';
  clearTimeout(debounce);void rpc('textDocument/didChange',{textDocument:{uri:currentUri,version:sourceVersion},contentChanges:[{text:editor.value}]},true);
  compilePending=new Promise(resolve=>{compileResolver=resolve;compiler.postMessage({kind:'compile',source:editor.value,optimize:$<HTMLInputElement>('optimize').checked,id:buildId++,version:sourceVersion});});return compilePending;
}
compiler.onmessage=(event:MessageEvent)=>{
  const m=event.data;
  if(m.kind==='rpc'){for(const message of m.messages as Rpc[]){if(message.id!==undefined){const pending=requests.get(message.id);requests.delete(message.id);if(message.error)pending?.reject(new Error(message.error.message));else pending?.resolve(message.result);}else if(message.method==='textDocument/publishDiagnostics'){const params=message.params as {uri:string;version?:number;diagnostics:Diagnostic[]};if(params.uri===currentUri&&(params.version===undefined||params.version===sourceVersion)){diagnostics=params.diagnostics;$('lsp-led').classList.add('ready');if(!busy&&builtVersion!==sourceVersion)messages(diagnostics);}}}}
  if(m.kind==='asset'){assetRequests.get(m.id)?.();assetRequests.delete(m.id);}
  if(m.kind==='compiled'){
    busy=false;$<HTMLSelectElement>('game-select').disabled=loadingGame;$<HTMLButtonElement>('build').disabled=false;$('build').innerHTML=`${icons.play} Build`;
    const report=m.report as Build;if(report.ok){build=report;rom=m.rom;builtVersion=m.version;$('dirty').textContent=builtVersion===sourceVersion?'':'●';$('build-time').textContent=`${m.elapsed.toFixed(1)} ms`;
      const o=report.optimizations;messages([],`✓ Real NES ROM · ${o.constantFolds} folds · ${o.inlined} inlined calls · ${o.machine} machine transforms`);
      const select=$<HTMLSelectElement>('pass-select');select.replaceChildren(...report.passes.map((p,i)=>{const option=document.createElement('option');option.value=String(i);option.textContent=p.name;return option;}));select.value=String(Math.min(passIndex,report.passes.length-1));
      $('ram-status').textContent=`RAM ${report.ramBytes} B`;$('zp-status').textContent=`ZERO PAGE ${report.zeroPageBytes} B`;$('rom-status').textContent=`ROM ${(report.romBytes/1024).toFixed(1)} KiB`;
      $('status').textContent='Build complete · compiled locally in Rust/WASM';renderInspector();cartridgeLoaded=false;frames=0;framePending=false;emulationVersion++;frameResolver?.();frameResolver=null;audioNode?.port.postMessage({reset:true});emulator.postMessage({kind:'load',id:emulationVersion,rom:rom!.slice(0),sampleRate:audioContext?.sampleRate??48000});
    }else{messages(report.diagnostics??[],report.error?`Build failed: ${report.error}`:undefined);$('status').textContent='Build failed · showing the previous cartridge';}
    compileResolver?.(report);compileResolver=null;
  }
  if(m.kind==='error'){busy=false;$<HTMLSelectElement>('game-select').disabled=loadingGame;$<HTMLButtonElement>('build').disabled=false;$('build').innerHTML=`${icons.play} Build`;messages([],`Compiler worker error: ${m.message}`);for(const pending of requests.values())pending.reject(new Error(m.message));requests.clear();compileResolver?.({ok:false,error:m.message} as Build);compileResolver=null;}
};
emulator.onmessage=(event:MessageEvent)=>{
  const m=event.data;if(m.id!==undefined&&m.id!==emulationVersion)return;
  if(m.kind==='loaded'){cartridgeLoaded=true;playing=true;lastFrame=performance.now();$('screen-loading').hidden=true;$('run-status').textContent='RUNNING';$('pause').innerHTML=icons.pause;}
  if(m.kind==='frame'){
    framePending=false;frames+=m.count;fpsFrames+=playing?m.count:0;ram=new Uint8Array(m.ram);
    const rgb=new Uint8Array(m.pixels);for(let i=0,j=0;i<rgb.length;i+=3,j+=4){image.data[j]=rgb[i];image.data[j+1]=rgb[i+1];image.data[j+2]=rgb[i+2];image.data[j+3]=255;}ctx.putImageData(image,0,0);
    if(sound&&audioNode&&playing)audioNode.port.postMessage({samples:new Float32Array(m.audio)},[m.audio]);
    $('pc').textContent=`PC $${m.pc.toString(16).toUpperCase().padStart(4,'0')}`;if(inspector==='memory')renderMemory();frameResolver?.();frameResolver=null;
  }
  if(m.kind==='error'){framePending=false;playing=false;frameResolver?.();frameResolver=null;messages([],`Emulator error: ${m.message}`);$('run-status').textContent='ERROR';}
};
function frame(count=1,input=buttons):void { if(cartridgeLoaded&&!framePending){framePending=true;emulator.postMessage({kind:'frame',id:emulationVersion,buttons:input,count});} }
function gamepad():number {const pad=navigator.getGamepads?.()[0];if(!pad)return 0;let v=0;for(const [i,mask]of [[0,1],[1,2],[8,4],[9,8],[12,16],[13,32],[14,64],[15,128]])if(pad.buttons[i]?.pressed)v|=mask;if(pad.axes[0]<-0.4)v|=64;if(pad.axes[0]>0.4)v|=128;if(pad.axes[1]<-0.4)v|=16;if(pad.axes[1]>0.4)v|=32;return v;}
function animate(now:number):void {
  if(playing&&cartridgeLoaded&&!document.hidden&&!framePending){const interval=1000/60.0988;const due=Math.floor((now-lastFrame)/interval);if(due>0){const count=Math.min(due,3);lastFrame=due>3?now:lastFrame+count*interval;frame(count,buttons|gamepad());}}
  if(now-fpsStart>=1000){$('fps').textContent=playing?`${Math.round(fpsFrames*1000/(now-fpsStart))} fps`:'paused';fpsFrames=0;fpsStart=now;}
  requestAnimationFrame(animate);
}
function setPlaying(value:boolean):void { playing=value;lastFrame=performance.now();$('pause').innerHTML=value?icons.pause:icons.play;$('run-status').textContent=value?'RUNNING':'PAUSED';audioNode?.port.postMessage({reset:true}); }
async function enableSound():Promise<void>{
  if(!audioContext){audioContext=new AudioContext({sampleRate:48000});await audioContext.audioWorklet.addModule(new URL('audio-worklet.js',document.baseURI));audioNode=new AudioWorkletNode(audioContext,'nes-audio');audioNode.connect(audioContext.destination);}
  await audioContext.resume();sound=!sound;$('sound').innerHTML=`♫ <span>${sound?'Sound on':'Sound off'}</span>`;$('sound').classList.toggle('enabled',sound);document.querySelector('.audio-bars')!.classList.toggle('active',sound);if(!sound)audioNode?.port.postMessage({reset:true});
}
async function loadGame(index:number):Promise<void>{
  if(index<0||index>=games.length)throw new Error('Unknown cartridge');
  if(busy||loadingGame)return;loadingGame=true;$<HTMLSelectElement>('game-select').disabled=true;clearTimeout(debounce);buttons=0;previousDraft=null;$('reset-source').textContent='Reset example';const oldUri=currentUri;gameIndex=index;const g=games[index];$<HTMLSelectElement>('game-select').value=g.id;const url=new URL(location.href);url.searchParams.set('game',g.id);history.replaceState(null,'',url);document.documentElement.style.setProperty('--game-accent',g.color);
  try {
  document.querySelectorAll('.game-tab').forEach((tab,i)=>tab.classList.toggle('active',i===index));$('filename').textContent=`${g.id}.nst`;$('game-title').textContent=g.name;$('genre-label').textContent=`0${index+1} / ${g.genre}`;$('game-description').textContent=g.description;$('controls').textContent=g.controls;$('song-name').textContent=g.song;$('mapper-badge').textContent=`${g.mapper} · NES`;
  if(oldUri)await rpc('textDocument/didClose',{textDocument:{uri:oldUri}},true);
  currentUri=`file:///games/${g.id}.nst`;let saved:string|null=null;try{saved=localStorage.getItem(`nested-source-${g.id}`);}catch{/* Optional persistence. */}
  const response=await fetch(new URL(`examples/${g.id}.nst`,document.baseURI));if(!response.ok)throw new Error(`Could not load ${g.id}.nst`);originalSource=await response.text();editor.value=saved??originalSource;sourceVersion++;builtVersion=-1;editor.scrollTop=0;editor.scrollLeft=0;editor.setSelectionRange(0,0);plainHighlight();updateCursor();
  await rpc('textDocument/didOpen',{textDocument:{uri:currentUri,languageId:'nested',version:sourceVersion,text:editor.value}},true);await compileSource();
  } finally {loadingGame=false;$<HTMLSelectElement>('game-select').disabled=false;}
}
function download(data:Blob,name:string):void {const a=document.createElement('a');a.href=URL.createObjectURL(data);a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000);}
$('build').onclick=()=>void compileSource();$('pause').onclick=()=>setPlaying(!playing);$('step').onclick=()=>{setPlaying(false);frame();};$('reset').onclick=()=>{emulator.postMessage({kind:'reset',id:emulationVersion});frames=0;audioNode?.port.postMessage({reset:true});};$('sound').onclick=()=>void enableSound().catch(e=>messages([],`Audio could not start: ${e}`));
$('download-rom').onclick=()=>{if(rom)download(new Blob([rom],{type:'application/octet-stream'}),`${games[gameIndex].id}.nes`);};$('save-source').onclick=()=>download(new Blob([editor.value],{type:'text/plain'}),`${games[gameIndex].id}.nst`);
$('format').onclick=()=>void rpc('textDocument/formatting',documentParams({options:{tabSize:4,insertSpaces:true}})).then(edits=>{applyEdits(edits as {range:Range;newText:string}[]);});
$<HTMLSelectElement>('pass-select').onchange=event=>{passIndex=Number((event.target as HTMLSelectElement).value);renderInspector();};
$<HTMLSelectElement>('game-select').onchange=event=>void loadGame(games.findIndex(g=>g.id===(event.target as HTMLSelectElement).value)).catch(e=>messages([],String(e)));
document.querySelectorAll<HTMLButtonElement>('[data-inspector]').forEach(tab=>tab.onclick=()=>{inspector=tab.dataset.inspector!;document.querySelectorAll('[data-inspector]').forEach(t=>t.classList.toggle('active',t===tab));renderInspector();});
editor.oninput=changed;editor.onscroll=syncScroll;editor.onclick=updateCursor;editor.onkeyup=updateCursor;
canvas.onkeydown=event=>{const key=event.key.toLowerCase();const mask=keyMask(key);if(mask){buttons|=mask;event.preventDefault();}if(key==='escape'){setPlaying(!playing);event.preventDefault();}};
canvas.onkeyup=event=>{const mask=keyMask(event.key.toLowerCase());if(mask){buttons&=~mask;event.preventDefault();}};canvas.onblur=()=>buttons=0;window.onblur=()=>{buttons=0;document.querySelectorAll('.pressed').forEach(b=>b.classList.remove('pressed'));audioNode?.port.postMessage({reset:true});};document.addEventListener('visibilitychange',()=>{buttons=0;lastFrame=performance.now();audioNode?.port.postMessage({reset:true});});
function keyMask(key:string):number{return({arrowup:16,arrowdown:32,arrowleft:64,arrowright:128,z:1,x:2,shift:4,enter:8}as Record<string,number>)[key]??0;}
document.querySelectorAll<HTMLButtonElement>('[data-button]').forEach(button=>{const mask=Number(button.dataset.button);button.onpointerdown=event=>{button.setPointerCapture(event.pointerId);buttons|=mask;button.classList.add('pressed');event.preventDefault();};const release=()=>{buttons&=~mask;button.classList.remove('pressed');};button.onlostpointercapture=release;button.onpointerup=release;button.onpointercancel=release;});
let completionItems:{label:string;detail?:string}[]=[];let completionIndex=0;
function showCompletions():void {const popup=$('completion');popup.hidden=false;popup.innerHTML=completionItems.map((item,i)=>`<button class="${i===completionIndex?'selected':''}" data-index="${i}"><strong>${escape(item.label)}</strong><small>${escape(item.detail??'')}</small></button>`).join('');popup.querySelectorAll<HTMLButtonElement>('button').forEach(button=>button.onmousedown=event=>{event.preventDefault();acceptCompletion(Number(button.dataset.index));});}
function acceptCompletion(index=completionIndex):void{const value=completionItems[index];if(!value)return;const end=editor.selectionStart;const prefix=editor.value.slice(0,end).match(/[a-zA-Z_][\w]*$/)?.[0]??'';editor.setRangeText(value.label,end-prefix.length,end,'end');$('completion').hidden=true;changed();}
async function complete():Promise<void>{const result=await rpc('textDocument/completion',documentParams({position:pos(editor.selectionStart)}))as{items:{label:string;detail?:string}[]};const prefix=editor.value.slice(0,editor.selectionStart).match(/[a-zA-Z_][\w]*$/)?.[0]??'';completionItems=result.items.filter(item=>item.label.startsWith(prefix)).slice(0,9);completionIndex=0;showCompletions();}
function applyEdits(edits:{range:Range;newText:string}[]):void {if(!edits?.length)return;const values=edits.map(e=>({start:off(e.range.start),end:off(e.range.end),text:e.newText})).sort((a,b)=>b.start-a.start);const selection=editor.selectionStart;let text=editor.value;for(const e of values)text=text.slice(0,e.start)+e.text+text.slice(e.end);editor.value=text;editor.setSelectionRange(Math.min(selection,text.length),Math.min(selection,text.length));changed();}
editor.onkeydown=event=>{
  if((event.ctrlKey||event.metaKey)&&event.key==='Enter'){event.preventDefault();void compileSource();}
  if((event.ctrlKey||event.metaKey)&&event.code==='Space'){event.preventDefault();void complete();}
  if((event.ctrlKey||event.metaKey)&&event.key==='s'){event.preventDefault();$('save-source').click();}
  if(event.shiftKey&&event.altKey&&event.key.toLowerCase()==='f'){event.preventDefault();$('format').click();}
  if(event.key==='F12'){event.preventDefault();void rpc('textDocument/definition',documentParams({position:pos(editor.selectionStart)})).then(v=>{if(v)goto((v as {range:Range}).range);});}
  if(event.key==='F2'){event.preventDefault();void rpc('textDocument/prepareRename',documentParams({position:pos(editor.selectionStart)})).then(v=>{if(v){$<HTMLInputElement>('rename-input').value=(v as {placeholder:string}).placeholder;$<HTMLDialogElement>('rename-dialog').showModal();}});}
  if(!$('completion').hidden){if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();completionIndex=(completionIndex+(event.key==='ArrowDown'?1:completionItems.length-1))%completionItems.length;showCompletions();}if(event.key==='Enter'||event.key==='Tab'){event.preventDefault();acceptCompletion();}if(event.key==='Escape')$('completion').hidden=true;}
  else if(event.key==='Enter'){event.preventDefault();const prefix=editor.value.slice(0,editor.selectionStart).split('\n').at(-1)!;const indent=prefix.match(/^\s*/)?.[0]??'';editor.setRangeText('\n'+indent+(prefix.trimEnd().endsWith('{')?'    ':''),editor.selectionStart,editor.selectionEnd,'end');changed();}
  else if(event.key==='Tab'){event.preventDefault();editor.setRangeText('    ',editor.selectionStart,editor.selectionEnd,'end');changed();}
};
$<HTMLDialogElement>('rename-dialog').onclose=()=>{if($<HTMLDialogElement>('rename-dialog').returnValue==='rename')void rpc('textDocument/rename',documentParams({position:pos(editor.selectionStart),newName:$<HTMLInputElement>('rename-input').value})).then(value=>{const changes=(value as{changes?:Record<string,{range:Range;newText:string}[]>})?.changes;applyEdits(changes?.[currentUri]??[]);}).catch(e=>messages([],String(e)));};
let hoverTimer:ReturnType<typeof setTimeout>;
editor.onmousemove=event=>{clearTimeout(hoverTimer);const bounds=editor.getBoundingClientRect();const line=Math.floor((event.clientY-bounds.top+editor.scrollTop-16)/21);const measure=document.createElement('canvas').getContext('2d')!;measure.font=getComputedStyle(editor).font;const width=measure.measureText('M').width;const character=Math.max(0,Math.floor((event.clientX-bounds.left+editor.scrollLeft-12)/width));hoverTimer=setTimeout(()=>{void rpc('textDocument/hover',documentParams({position:{line:Math.max(0,line),character}})).then(value=>{const hover=$('hover');if(value){const text=(value as{contents:{value:string}}).contents.value;hover.textContent=text.replace(/```nested\n|```/g,'');hover.hidden=false;}else hover.hidden=true;});},350);};editor.onmouseleave=()=>{clearTimeout(hoverTimer);$('hover').hidden=true;};
$<HTMLInputElement>('assets').onchange=async event=>{for(const file of Array.from((event.target as HTMLInputElement).files??[])){const id=assetId++;const data=await file.arrayBuffer();await new Promise<void>(resolve=>{assetRequests.set(id,resolve);compiler.postMessage({kind:'asset',id,name:`assets/${file.name}`,data},[data]);});}messages([], 'Custom assets loaded. Build to use them.');};
async function initialize():Promise<void>{
  const response=await fetch(new URL('nested.wasm',document.baseURI));if(!response.ok)throw new Error('Build the WASM module with npm run build');const module=await WebAssembly.compile(await response.arrayBuffer());
  await Promise.all([compiler,emulator].map(worker=>new Promise<void>((resolve,reject)=>{const clean=()=>{clearTimeout(timer);worker.removeEventListener('message',ready);worker.removeEventListener('error',failed);};const failed=(event:ErrorEvent)=>{clean();reject(new Error(event.message||'Worker failed to initialize'));};const ready=(event:MessageEvent)=>{if(event.data.kind==='ready'){clean();resolve();}else if(event.data.kind==='error'){clean();reject(new Error(event.data.message));}};const timer=setTimeout(()=>{clean();reject(new Error('Worker initialization timed out'));},30000);worker.addEventListener('message',ready);worker.addEventListener('error',failed);worker.postMessage({kind:'init',module});})));
  await rpc('initialize',{processId:null,rootUri:'file:///games',capabilities:{general:{positionEncodings:['utf-16']}}});await rpc('initialized',{},true);
  const assets=['bloom.chr','bloom.pal','bloom.map','bloom.puzzles.bin','bloom.clues.bin','starstring.chr','starstring.pal','starstring.map','starstring.chart.bin','skythread.chr','skythread.pal','skythread.rooms.bin','skythread.collision.bin','emberkeep.chr','emberkeep.pal','emberkeep.map'];
  for(let i=0;i<6;i++)assets.push(`skythread-${i}.map`);
  await Promise.all(assets.map(async name=>{const r=await fetch(new URL(`assets/${name}`,document.baseURI));if(!r.ok)throw new Error(`Missing ${name}`);const data=await r.arrayBuffer();const id=assetId++;return new Promise<void>(resolve=>{assetRequests.set(id,resolve);compiler.postMessage({kind:'asset',name:`assets/${name}`,data,id},[data]);});}));
  const requested=new URL(location.href).searchParams.get('game');const index=games.findIndex(g=>g.id===requested);await loadGame(index<0?0:index);requestAnimationFrame(animate);
}
// Compact panes stay available at every width; desktop dividers are draggable.
app.dataset.layout='workbench';app.dataset.pane='editor';
let paneSizes:number[]|null=null;
function fitScreen():void{const content=document.querySelector<HTMLElement>('.player-content')!;const shell=document.querySelector<HTMLElement>('.screen-shell')!;const scale=Math.max(1,Math.min(3,Math.floor(Math.min(content.clientWidth-24,(content.clientHeight-160)*256/240)/256)));shell.style.width=`${scale*256+2}px`;}
new ResizeObserver(fitScreen).observe(document.querySelector('.player-content')!);
function setLayout(value:string):void{app.dataset.layout=value;paneSizes=null;document.querySelector<HTMLElement>('.workspace')!.style.removeProperty('grid-template-columns');try{localStorage.setItem('nested-layout',value);}catch{}requestAnimationFrame(fitScreen);}
$<HTMLSelectElement>('layout-select').onchange=event=>setLayout((event.target as HTMLSelectElement).value);
document.querySelectorAll<HTMLButtonElement>('[data-pane]').forEach(button=>button.onclick=()=>{app.dataset.pane=button.dataset.pane;document.querySelectorAll<HTMLButtonElement>('[data-pane]').forEach(b=>{b.classList.toggle('active',b===button);b.setAttribute('aria-pressed',String(b===button));});requestAnimationFrame(fitScreen);});
function resizePane(index:number,delta:number):void{const work=document.querySelector<HTMLElement>('.workspace')!;const editorPanel=document.querySelector<HTMLElement>('.editor-panel')!;const playerPanel=document.querySelector<HTMLElement>('.player-panel')!;const width=work.clientWidth-12;if(app.dataset.layout!=='workbench')return;const sizes=paneSizes??[editorPanel.getBoundingClientRect().width,playerPanel.getBoundingClientRect().width];if(index===0)sizes[0]=Math.max(260,Math.min(width-sizes[1]-220,sizes[0]+delta));else sizes[1]=Math.max(276,Math.min(width-sizes[0]-220,sizes[1]+delta));paneSizes=sizes;work.style.gridTemplateColumns=`${sizes[0]}px 6px ${sizes[1]}px 6px minmax(0,1fr)`;}
document.querySelectorAll<HTMLElement>('[data-divider]').forEach(handle=>{const index=Number(handle.dataset.divider);handle.onpointerdown=event=>{handle.setPointerCapture(event.pointerId);let last=event.clientX;handle.onpointermove=e=>{resizePane(index,e.clientX-last);last=e.clientX;};handle.onpointerup=()=>handle.onpointermove=null;};handle.onkeydown=event=>{if(['ArrowLeft','ArrowRight'].includes(event.key)){event.preventDefault();resizePane(index,event.key==='ArrowLeft'?-24:24);}};});
window.addEventListener('resize',()=>{paneSizes=null;document.querySelector<HTMLElement>('.workspace')!.style.removeProperty('grid-template-columns');});
$('reset-source').onclick=()=>{if(previousDraft===null){previousDraft=editor.value;editor.value=originalSource;$('reset-source').textContent='Restore draft';}else{editor.value=previousDraft;previousDraft=null;$('reset-source').textContent='Reset example';}changed();void compileSource();};
$<HTMLInputElement>('source-file').onchange=async event=>{const input=event.target as HTMLInputElement;const file=input.files?.[0];if(file){if(file.size>2000000){messages([],'Source exceeds 2 MB');return;}previousDraft=editor.value;$('reset-source').textContent='Restore draft';editor.value=await file.text();changed();void compileSource();}input.value='';};
try{const layout=localStorage.getItem('nested-layout');if(layout&&['workbench','play','code'].includes(layout)){$<HTMLSelectElement>('layout-select').value=layout;setLayout(layout);}}catch{}
// Inspectable workbench API, also used by the browser integration test.
Object.assign(window,{__nested:{get build(){return build;},get ram(){return ram;},get frames(){return frames;},get source(){return editor.value;},get ready(){return cartridgeLoaded;},rpc,loadGame,compile:compileSource,pause:()=>setPlaying(false),play:()=>setPlaying(true),setSource:(s:string)=>{editor.value=s;changed();},step:async(input=0,count=1)=>{setPlaying(false);while(framePending)await new Promise(r=>setTimeout(r,5));return new Promise<void>(resolve=>{frameResolver=resolve;frame(count,input);});},get canvas(){return canvas;}}});
void initialize().catch(error=>{messages([],String(error));$('screen-loading').textContent='The workbench could not start';$('status').textContent='Startup error';});
