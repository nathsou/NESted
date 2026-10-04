import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {Browser} from './browser.mjs';
const server=spawn('npm',['exec','vite','--','preview','playground','--host','127.0.0.1','--port','4173'],{stdio:'ignore'});
const browser=new Browser();
try{
 const start=Date.now();for(;;){try{if((await fetch('http://127.0.0.1:4173/')).ok)break;}catch{}if(Date.now()-start>30000)throw new Error('Preview server startup failed');await new Promise(r=>setTimeout(r,100));}
 await browser.start('http://127.0.0.1:4173/');await browser.wait('window.__nested?.ready && window.__nested.frames>20');
 await browser.evaluate('window.__nested.pause()');
 for(let i=0;i<4;i++){
  if(i>0){await browser.evaluate(`window.__nested.loadGame(${i})`);await browser.wait(`window.__nested.ready && document.getElementById('filename').textContent.includes('${['bloom','starstring','skythread','emberkeep'][i]}')`);await browser.evaluate('window.__nested.pause()');}
  await browser.evaluate('window.__nested.step(0,120)');
  const report=await browser.evaluate('({ok:window.__nested.build.ok,mapper:window.__nested.build.mapper,passes:window.__nested.build.passes.length,drop:window.__nested.ram[895]})');assert.equal(report.ok,true);assert.equal(report.mapper,[0,4,1,2][i]);assert.equal(report.passes,7);assert.equal(report.drop,0);
 }
 const layout=await browser.evaluate(`({header:document.querySelector('.header').getBoundingClientRect().height,select:document.getElementById('game-select').getBoundingClientRect().height,screen:document.getElementById('screen').getBoundingClientRect().width,overflow:document.documentElement.scrollWidth>innerWidth})`);assert.equal(layout.header,44);assert.ok(layout.select<34);assert.equal(layout.screen%256,0);assert.equal(layout.overflow,false);
 const before=await browser.evaluate(`document.querySelector('.editor-panel').getBoundingClientRect().width`);
 await browser.evaluate(`document.querySelector('[data-divider="0"]').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}))`);
 assert.equal(await browser.evaluate(`document.querySelector('.editor-panel').getBoundingClientRect().width`),before+24);
 for(const value of ['play','code','workbench']){await browser.evaluate(`(()=>{const select=document.getElementById('layout-select');select.value='${value}';select.dispatchEvent(new Event('change'));})()`);assert.equal(await browser.evaluate(`document.getElementById('app').dataset.layout`),value);}
 await browser.send('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
 for(const pane of ['editor','player','inspector']){await browser.evaluate(`document.querySelector('[data-pane="${pane}"]').click()`);assert.equal(await browser.evaluate(`getComputedStyle(document.querySelector('.${pane==='editor'?'editor':pane==='player'?'player':'inspector'}-panel')).display`),'flex');assert.equal(await browser.evaluate('document.documentElement.scrollWidth>innerWidth'),false);}
 for(const width of [320,390,820,1024,1366]){await browser.send('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:width<801});assert.equal(await browser.evaluate('document.documentElement.scrollWidth>innerWidth'),false,`Viewport ${width} overflows`);}
 await browser.send('Emulation.setDeviceMetricsOverride',{width:1600,height:1000,deviceScaleFactor:1,mobile:false});
 const rpc=await browser.evaluate(`window.__nested.rpc('textDocument/completion',{textDocument:{uri:'file:///games/emberkeep.nst'},position:{line:31,character:0}})`);assert.ok(rpc.items.some(i=>i.label==='tone'));
 const original=await browser.evaluate('window.__nested.source');
 // Formatting immediately after an edit must flush didChange before querying the LSP.
 await browser.evaluate(`window.__nested.setSource('var answer:u8=0;fn update(){answer=match buttons(){0=>1,_=>2};}')`);
 const formatted=await browser.evaluate(`window.__nested.rpc('textDocument/formatting',{textDocument:{uri:'file:///games/emberkeep.nst'},options:{tabSize:4,insertSpaces:true}})`);assert.match(formatted[0].newText,/    answer = match buttons\(\)/);assert.match(formatted[0].newText,/0 => 1,\n/);
 await browser.evaluate(`window.__nested.setSource(${JSON.stringify(original+'\n// locally edited draft\n')})`);
 await browser.evaluate(`document.getElementById('reset-source').click()`);await browser.wait(`document.getElementById('build').disabled===false`);assert.equal(await browser.evaluate(`window.__nested.source.includes('locally edited draft')`),false);
 await browser.evaluate(`document.getElementById('reset-source').click()`);await browser.wait(`document.getElementById('build').disabled===false`);assert.equal(await browser.evaluate(`window.__nested.source.includes('locally edited draft')`),true);
 await browser.evaluate(`window.__nested.setSource('fn update(){ unknown(); }')`);const failed=await browser.evaluate('window.__nested.compile()');assert.equal(failed.ok,false);assert.match(await browser.evaluate('document.getElementById("messages").textContent'),/Unknown function/);
 await browser.evaluate(`window.__nested.setSource(${JSON.stringify(original)})`);assert.equal((await browser.evaluate('window.__nested.compile()')).ok,true);await browser.wait('window.__nested.ready');
 await browser.evaluate('document.getElementById("sound").click()');await browser.wait('document.getElementById("sound").textContent.includes("Sound on")');
 assert.equal(browser.errors.length,0,JSON.stringify(browser.errors));
 console.log('Production browser: four cartridges, assets under a relative base, LSP, diagnostics, rebuild, audio, compact layouts, resizing, mobile panes, fresh-document formatting and draft restoration passed.');
}finally{await browser.close();server.kill('SIGTERM');}
