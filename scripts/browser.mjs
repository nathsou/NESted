// Dependency-free Chromium DevTools client, used for screenshots and UI tests.
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
export class Browser {
  constructor() { this.profile=mkdtempSync(join(tmpdir(),'nested-browser-'));this.id=1;this.pending=new Map();this.errors=[]; }
  async start(url='http://127.0.0.1:5173/',width=1600,height=1000) {
    this.process=spawn(process.env.CHROMIUM??'chromium',['--headless=new','--no-sandbox','--disable-dev-shm-usage','--remote-debugging-port=0',`--user-data-dir=${this.profile}`,`--window-size=${width},${height}`,'--force-device-scale-factor=1','--autoplay-policy=no-user-gesture-required','about:blank'],{stdio:['ignore','ignore','pipe']});
    const endpoint=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error('Chromium startup timed out')),20000);let text='';this.process.stderr.on('data',chunk=>{text+=chunk;const match=text.match(/DevTools listening on (ws:\/\/[^\s]+)/);if(match){clearTimeout(timer);resolve(match[1]);}});this.process.on('error',reject);});
    const port=new URL(endpoint).port;const response=await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent('about:blank')}`,{method:'PUT'});const tab=await response.json();this.socket=new WebSocket(tab.webSocketDebuggerUrl);
    await new Promise((resolve,reject)=>{this.socket.onopen=resolve;this.socket.onerror=reject;});
    this.socket.onmessage=event=>{const message=JSON.parse(event.data);if(message.id){const p=this.pending.get(message.id);this.pending.delete(message.id);if(message.error)p?.reject(new Error(message.error.message));else p?.resolve(message.result);}else if(message.method==='Runtime.exceptionThrown')this.errors.push(message.params.exceptionDetails);};
    await this.send('Runtime.enable');await this.send('Page.enable');await this.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await this.send('Page.navigate',{url});return this;
  }
  send(method,params={}) { const id=this.id++;return new Promise((resolve,reject)=>{this.pending.set(id,{resolve,reject});this.socket.send(JSON.stringify({id,method,params}));}); }
  async evaluate(expression) { const result=await this.send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(result.exceptionDetails)throw new Error(JSON.stringify(result.exceptionDetails));return result.result.value; }
  async wait(expression,timeout=45000) { const start=Date.now();while(Date.now()-start<timeout){if(await this.evaluate(expression))return;await new Promise(resolve=>setTimeout(resolve,100));}throw new Error(`Timed out waiting for ${expression}\n${await this.evaluate('document.getElementById("messages")?.textContent')}`); }
  async screenshot(path) { const shot=await this.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(path,Buffer.from(shot.data,'base64')); }
  async close() { this.socket?.close();this.process?.kill('SIGTERM');await new Promise(resolve=>setTimeout(resolve,300));rmSync(this.profile,{recursive:true,force:true}); }
}
