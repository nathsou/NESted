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
 const rpc=await browser.evaluate(`window.__nested.rpc('textDocument/completion',{textDocument:{uri:'file:///games/emberkeep.nst'},position:{line:31,character:0}})`);assert.ok(rpc.items.some(i=>i.label==='tone'));
 const original=await browser.evaluate('window.__nested.source');
 await browser.evaluate(`window.__nested.setSource('fn update(){ unknown(); }')`);const failed=await browser.evaluate('window.__nested.compile()');assert.equal(failed.ok,false);assert.match(await browser.evaluate('document.getElementById("messages").textContent'),/Unknown function/);
 await browser.evaluate(`window.__nested.setSource(${JSON.stringify(original)})`);assert.equal((await browser.evaluate('window.__nested.compile()')).ok,true);await browser.wait('window.__nested.ready');
 await browser.evaluate('document.getElementById("sound").click()');await browser.wait('document.getElementById("sound").textContent.includes("Sound on")');
 assert.equal(browser.errors.length,0,JSON.stringify(browser.errors));
 console.log('Production browser: four cartridges, assets under a relative base, LSP, diagnostics, rebuild, and audio activation passed.');
}finally{await browser.close();server.kill('SIGTERM');}
