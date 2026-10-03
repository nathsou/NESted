import {Browser} from './browser.mjs';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
const root=resolve(import.meta.dirname,'..'),out=resolve(root,'docs/screenshots');mkdirSync(out,{recursive:true});
const b=new Browser();
const names=['bloom','starstring','skythread','emberkeep'];
try{
 await b.start(process.env.PLAYGROUND_URL??'http://127.0.0.1:5173/',1600,1100);await b.wait('window.__nested?.ready && window.__nested.frames>20');
 for(let index=0;index<4;index++){
  if(index){await b.evaluate(`window.__nested.loadGame(${index})`);await b.wait(`window.__nested.ready && document.getElementById('filename').textContent==='${names[index]}.nst'`);}
  await b.evaluate('window.__nested.step(0,120)');
  if(index===0){const solution=[...readFileSync(resolve(root,'games/assets/bloom.puzzles.bin')).subarray(0,64)];await b.evaluate(`(async()=>{const n=window.__nested;const tap=async(p)=>{await n.step(p,3);await n.step(0,3);};const solution=${JSON.stringify(solution)};for(let y=0;y<8;y++){for(let i=0;i<8;i++){const x=y%2===0?i:7-i;if(solution[y*8+x])await tap(1);else await tap(2);if(i<7)await tap(y%2===0?128:64);}if(y<7)await tap(32);}await n.step(0,20);})()`);}
  if(index===1){await b.evaluate(`(async()=>{const n=window.__nested;await n.step(8,3);await n.step(0,3);const read=(name,i=0)=>n.ram[n.build.symbols['__v_'+name]+i];for(let f=0;f<430;f++){let held=0;for(let i=0;i<12;i++){const y=read('note_y',i);if(y>=188&&y<=190)held|=read('note_mask',i);if(read('sustain',i))held|=read('sustain_mask',i);}const pad=((held&1)<<6)|((held&2)<<4)|((held&4)<<2)|((held&8)<<4);await n.step(pad,1);}})()`);}
  if(index===2){await b.evaluate('(async()=>{await window.__nested.step(128,10);await window.__nested.step(129,10);await window.__nested.step(128,6);})()');}
  if(index===3){await b.evaluate(`(async()=>{const n=window.__nested;const read=(name,i=0)=>n.ram[n.build.symbols['__v_'+name]+i];for(let turn=0;turn<18;turn++){const cells=Array.from({length:192},(_,i)=>read('dungeon',i));const goal=cells.indexOf(read('key_found')?2:3),start=read('player_y')*16+read('player_x');if(read('key_found')&&read('player_x')>=8)break;const parents=Array(192).fill(null),queue=[start];parents[start]=[start,0];for(let at=0;at<queue.length;at++){const i=queue[at];if(i===goal)break;for(const[dx,dy,pad]of[[0,1,32],[0,-1,16],[1,0,128],[-1,0,64]]){const x=i%16+dx,y=Math.floor(i/16)+dy;if(x>=0&&x<16&&y>=0&&y<12){const j=y*16+x;if(cells[j]&&parents[j]===null){parents[j]=[i,pad];queue.push(j);}}}}let next=goal;while(parents[next][0]!==start)next=parents[next][0];await n.step(parents[next][1],3);await n.step(0,5);}await n.step(0,40);})()`);}
  const canvas=await b.evaluate('window.__nested.canvas.toDataURL("image/png").split(",")[1]');writeFileSync(resolve(out,`${names[index]}.png`),Buffer.from(canvas,'base64'));
  // Show the game loop alongside the actual executing ROM and machine listing.
  await b.evaluate(`(()=>{const editor=document.getElementById('source');const line=editor.value.slice(0,editor.value.indexOf('fn update')).split('\\n').length;editor.scrollTop=(line-1)*21;editor.dispatchEvent(new Event('scroll'));})()`);
  await b.screenshot(resolve(out,`${names[index]}-workbench.png`));if(index===0)await b.screenshot(resolve(out,'workbench.png'));console.log(`${names[index]} captured from compiled ROM`);
 }
 assertNoErrors();
 function assertNoErrors(){if(b.errors.length)throw new Error(JSON.stringify(b.errors));}
}finally{await b.close();}
