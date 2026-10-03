// VSIX is an OPC zip. Package with Node built-ins; no marketplace tooling dependency.
import {readFileSync,writeFileSync,mkdirSync,readdirSync,copyFileSync,existsSync} from 'node:fs';
import {resolve,relative} from 'node:path';
import {deflateRawSync} from 'node:zlib';
const root=resolve(import.meta.dirname,'..'),dir=resolve(root,'extension');
const platform=process.platform==='win32'?'win32':process.platform;
const target=`${platform}-${process.arch}`,binary=platform==='win32'?'nested.exe':'nested';
const native=resolve(root,'target/release',binary);
if(!existsSync(native))throw new Error('Run npm run build before packaging the native language server.');
mkdirSync(resolve(dir,'bin'),{recursive:true});copyFileSync(native,resolve(dir,'bin',binary));
const manifest=JSON.parse(readFileSync(resolve(dir,'package.json'),'utf8'));
const files=[];function add(name,data,executable=false){files.push({name,data:Buffer.from(data),executable});}
function walk(path){for(const entry of readdirSync(path,{withFileTypes:true})){if(entry.name==='dist')continue;const full=resolve(path,entry.name);if(entry.isDirectory())walk(full);else add(`extension/${relative(dir,full).replaceAll('\\','/')}`,readFileSync(full),full.endsWith(`/bin/${binary}`));}}
walk(dir);add('extension/LICENSE',readFileSync(resolve(root,'LICENSE')));
add('[Content_Types].xml','<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="json" ContentType="application/json"/><Default Extension="vsixmanifest" ContentType="text/xml"/><Default Extension="xml" ContentType="text/xml"/><Default Extension="cjs" ContentType="application/javascript"/><Default Extension="md" ContentType="text/plain"/><Default Extension="nst" ContentType="text/plain"/><Default Extension="exe" ContentType="application/octet-stream"/><Override PartName="/extension/bin/nested" ContentType="application/octet-stream"/><Override PartName="/extension/LICENSE" ContentType="text/plain"/></Types>');
add('extension.vsixmanifest',`<?xml version="1.0"?><PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011"><Metadata><Identity Language="en-US" Id="${manifest.name}" Version="${manifest.version}" Publisher="${manifest.publisher}" TargetPlatform="${target}"/><DisplayName>${manifest.displayName}</DisplayName><Description xml:space="preserve">${manifest.description}</Description><Tags>NES,6502,language</Tags><Categories>Programming Languages</Categories><Properties><Property Id="Microsoft.VisualStudio.Code.Engine" Value="${manifest.engines.vscode}"/></Properties><License>extension/LICENSE</License></Metadata><Installation><InstallationTarget Id="Microsoft.VisualStudio.Code"/></Installation><Dependencies/><Assets><Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true"/></Assets></PackageManifest>`);
function crc32(data){let crc=0xffffffff;for(const b of data){crc^=b;for(let bit=0;bit<8;bit++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}return (crc^0xffffffff)>>>0;}
let offset=0;const local=[],central=[];
for(const f of files){const name=Buffer.from(f.name),compressed=deflateRawSync(f.data),crc=crc32(f.data);const h=Buffer.alloc(30);h.writeUInt32LE(0x04034b50);h.writeUInt16LE(20,4);h.writeUInt16LE(8,8);h.writeUInt16LE(33,12);h.writeUInt32LE(crc,14);h.writeUInt32LE(compressed.length,18);h.writeUInt32LE(f.data.length,22);h.writeUInt16LE(name.length,26);local.push(h,name,compressed);
const c=Buffer.alloc(46);c.writeUInt32LE(0x02014b50);c.writeUInt16LE(0x0314,4);c.writeUInt16LE(20,6);c.writeUInt16LE(8,10);c.writeUInt16LE(33,14);c.writeUInt32LE(crc,16);c.writeUInt32LE(compressed.length,20);c.writeUInt32LE(f.data.length,24);c.writeUInt16LE(name.length,28);c.writeUInt32LE(((f.executable?0o100755:0o100644)<<16)>>>0,38);c.writeUInt32LE(offset,42);central.push(c,name);offset+=h.length+name.length+compressed.length;}
const directory=Buffer.concat(central),end=Buffer.alloc(22);end.writeUInt32LE(0x06054b50);end.writeUInt16LE(files.length,8);end.writeUInt16LE(files.length,10);end.writeUInt32LE(directory.length,12);end.writeUInt32LE(offset,16);
mkdirSync(resolve(dir,'dist'),{recursive:true});const output=resolve(dir,'dist',`nested-${manifest.version}-${target}.vsix`);writeFileSync(output,Buffer.concat([...local,directory,end]));console.log(output);
