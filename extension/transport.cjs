'use strict';
const {spawn}=require('node:child_process');
class LspTransport {
  constructor(binary,args=['lsp'],options={}) {
    this.sequence=1;this.pending=new Map();this.buffer=Buffer.alloc(0);this.onNotification=()=>{};this.onLog=()=>{};
    this.process=spawn(binary,args,{stdio:['pipe','pipe','pipe'],...options});
    this.process.stdout.on('data',data=>this.read(data));this.process.stderr.on('data',data=>this.onLog(data.toString()));
    this.process.on('error',error=>{this.onLog(String(error));this.fail(error);});
    this.process.on('exit',(code,signal)=>this.fail(new Error(`Language server exited (${code??signal})`)));
  }
  read(data) {
    this.buffer=Buffer.concat([this.buffer,data]);
    for(;;){const end=this.buffer.indexOf('\r\n\r\n');if(end<0){if(this.buffer.length>8192)this.fail(new Error('Invalid LSP framing'));return;}
      const header=this.buffer.subarray(0,end).toString();const match=header.match(/(?:^|\r\n)Content-Length:\s*(\d+)/i);if(!match){this.fail(new Error('Missing LSP length'));return;}
      const length=Number(match[1]);if(length>16*1024*1024){this.fail(new Error('LSP response exceeds limit'));return;}if(this.buffer.length<end+4+length)return;
      const body=this.buffer.subarray(end+4,end+4+length);this.buffer=this.buffer.subarray(end+4+length);
      try{const message=JSON.parse(body.toString());if(message.id!==undefined){const item=this.pending.get(message.id);this.pending.delete(message.id);if(item){clearTimeout(item.timer);if(message.error)item.reject(new Error(message.error.message));else item.resolve(message.result);}}else this.onNotification(message.method,message.params);}catch(error){this.onLog(String(error));}
    }
  }
  write(message){if(this.process.stdin.destroyed)throw new Error('Language server is not running');const body=Buffer.from(JSON.stringify({jsonrpc:'2.0',...message}));this.process.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);this.process.stdin.write(body);}
  notify(method,params){this.write({method,params});}
  request(method,params){const id=this.sequence++;return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{this.pending.delete(id);reject(new Error(`LSP request timed out: ${method}`));},15000);timer.unref();this.pending.set(id,{resolve,reject,timer});try{this.write({id,method,params});}catch(error){clearTimeout(timer);this.pending.delete(id);reject(error);}});}
  fail(error){for(const item of this.pending.values()){clearTimeout(item.timer);item.reject(error);}this.pending.clear();}
  async close(){try{await this.request('shutdown',null);this.notify('exit',null);}catch{}this.process.stdin.end();const timer=setTimeout(()=>this.process.kill(),1000);timer.unref();}
}
module.exports={LspTransport};
