// Run actual original metadata-connect methods against synthetic redistributable files.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {createRequire} from 'node:module';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
import {execFileSync} from 'node:child_process';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url),ts=require(process.env.METADATA_TYPESCRIPT||'typescript');
const root=process.env.METADATA_SOURCE?path.resolve(process.env.METADATA_SOURCE):fileURLToPath(new URL('../../../packages/metadata-connect/',import.meta.url));
const revision=execFileSync('git',['-C',root,'rev-parse','HEAD'],{encoding:'utf8'}).trim();
assert.equal(revision,'c2ae262650a7bbe7af5fb5222a6ab6b882ecf865','Original source revision drift');
const sourceTests=[];
const it=(name,run)=>sourceTests.push({name,run});
it.each=values=>(name,run)=>{for(const value of values)it(name.replace('%s',value),()=>run(value));};
const vitest={describe:(_name,run)=>run(),it,expect:value=>({toBe:expected=>assert.equal(value,expected),toBeUndefined:()=>assert.equal(value,undefined),toBeNull:()=>assert.equal(value,null),toContain:expected=>assert.ok(value.includes(expected)),toBeGreaterThan:expected=>assert.ok(value>expected),toBeTypeOf:expected=>assert.equal(typeof value,expected)})};
const cache=new Map();
async function load(file){if(cache.has(file))return cache.get(file);const source=await readFile(path.join(root,file),'utf8');const compiled=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;const module={exports:{}};cache.set(file,module.exports);const imports=[...compiled.matchAll(/require\("([^"\n]+)"\)/g)].map(m=>m[1]);const deps=new Map();for(const id of imports){if(id==='vitest'){deps.set(id,vitest);continue;}if(!id.startsWith('.'))throw Error(`Forbidden native import ${id}`);deps.set(id,await load(path.posix.normalize(path.posix.join(path.posix.dirname(file),id.replace(/\.js$/,'.ts')))));}vm.runInNewContext(`(function(require,module,exports,Buffer){${compiled}\n})`,{},{filename:file})(id=>{if(!deps.has(id))throw Error(`Unexpected ${id}`);return deps.get(id);},module,module.exports,Buffer);cache.set(file,module.exports);return module.exports;}
const api=await load('src/index.ts'),utils=await load('src/parsers/utils.ts');
await load('test/extract.test.ts');await load('test/label.test.ts');for(const test of sourceTests)await test.run();
const be=n=>{const b=Buffer.alloc(4);b.writeUInt32BE(n);return b;},le=n=>{const b=Buffer.alloc(4);b.writeUInt32LE(n);return b;},sync=n=>Buffer.from([(n>>21)&127,(n>>14)&127,(n>>7)&127,n&127]);
function frame(id,body,v=3){if(v===2){const h=Buffer.alloc(6);h.write(id);h.writeUIntBE(body.length,3,3);return Buffer.concat([h,body]);}return Buffer.concat([Buffer.from(id),v===4?sync(body.length):be(body.length),Buffer.alloc(2),body]);}
function tag(frames,v=3){const body=Buffer.concat(frames);return Buffer.concat([Buffer.from([73,68,51,v,0,0]),sync(body.length),body]);}
const text=(id,value,v=3,encoding=3)=>frame(id,Buffer.concat([Buffer.from([encoding]),Buffer.from(value,encoding===0?'latin1':'utf8')]),v);
const image=Buffer.from([137,80,78,71,1,2,3,4]);
const apic=(kind,bytes=image)=>frame('APIC',Buffer.concat([Buffer.from([0]),Buffer.from('image/png\0'),Buffer.from([kind,0]),bytes]));
function atom(type,...body){const data=Buffer.concat(body);return Buffer.concat([be(data.length+8),Buffer.from(type,'latin1'),data]);}
const datum=bytes=>atom('data',Buffer.alloc(8),bytes),free=(name,value)=>atom('----',atom('mean',Buffer.alloc(4),Buffer.from('com.apple.iTunes')),atom('name',Buffer.alloc(4),Buffer.from(name)),datum(Buffer.from(value)));
const mp4=(...items)=>Buffer.concat([atom('ftyp',Buffer.from('M4A \0\0\0\0')),atom('moov',atom('udta',atom('meta',Buffer.alloc(4),atom('ilst',...items))))]);
function comments(fields){const body=Buffer.concat([le(4),Buffer.from('test'),le(fields.length),...fields.flatMap(s=>{const b=Buffer.from(s);return[le(b.length),b];})]);return block(4,body);}
function block(kind,body,last=true){return Buffer.concat([Buffer.from([(last?128:0)|kind,(body.length>>16)&255,(body.length>>8)&255,body.length&255]),body]);}
function picture(kind,bytes=image){const mime=Buffer.from('image/png');return block(6,Buffer.concat([be(kind),be(mime.length),mime,be(0),be(1),be(1),be(24),be(0),be(bytes.length),bytes]));}
function aiff(id3){const chunk=Buffer.concat([Buffer.from('ID3 '),be(id3.length),id3]);const body=Buffer.concat([Buffer.from('AIFF'),chunk]);return Buffer.concat([Buffer.from('FORM'),be(body.length),body]);}
const cases=[];function add(name,extension,bytes,nativeExpected){cases.push({name,extension,bytes,nativeExpected});}
add('id3-v23-all','mp3',tag([text('TIT2',' Strobe\0 '),text('TPE1','Artist'),text('TALB','Album'),text('TCON','(17)'),text('TPUB','Anjunadeep'),text('TYER','2024-01-01'),text('TBPM','128.56 BPM'),text('TKEY','8A'),apic(4,Buffer.from([1,2,3,4])),apic(3)]));
add('id3-v22-label','mp3',tag([text('TPB','Drumcode',2)],2));
add('id3-v24-unicode','mp3',tag([text('TIT2','Déjà 🎧',4),text('TPUB','Warp',4),text('TDRC','2001',4)],4));
const utf16=Buffer.concat([Buffer.from([1,255,254]),Buffer.from('雪 🎧\0ignored','utf16le')]);add('id3-utf16le','mp3',tag([frame('TIT2',utf16)]));
const utfbe=Buffer.from('雪 🎧\0','utf16le');utfbe.swap16();add('id3-utf16be','mp3',tag([frame('TIT2',Buffer.concat([Buffer.from([1,254,255]),utfbe]))]));
add('id3-latin1','mp3',tag([text('TPE1','Beyoncé',3,0)]));
add('id3-v22-pic-corrected','mp3',tag([frame('PIC',Buffer.concat([Buffer.from([0]),Buffer.from('PNG'),Buffer.from([3,0]),image]),2)],2),{artwork:[...image],artworkMimeType:'image/png'});
add('aiff-label','aiff',aiff(tag([text('TPUB','Hyperdub')])));
for(const name of ['LABEL','PUBLISHER','ORGANIZATION','ORGANISATION'])add(`flac-${name.toLowerCase()}`,'flac',Buffer.concat([Buffer.from('fLaC'),comments([`${name}=Kompakt`])]));
add('flac-all','flac',Buffer.concat([Buffer.from('fLaC'),comments(['TITLE=Title','ARTIST=Artist','ALBUM=Album','GENRE=House','LABEL=Warp','DATE=2022','BPM=127.26','KEY=Am'])]));
add('flac-label-priority','flac',Buffer.concat([Buffer.from('fLaC'),comments(['ORGANIZATION=Distributor','LABEL=Warp'])]));
add('flac-artwork','flac',Buffer.concat([Buffer.from('fLaC'),picture(3)]));
for(const name of ['LABEL','PUBLISHER','ORGANIZATION'])add(`mp4-${name.toLowerCase()}`,'m4a',mp4(free(name,'Defected')));
add('mp4-freeform-filter','m4a',mp4(free('MOOD','Peak Time'),free('LABEL','Innervisions'),free('ENERGY','8')));
add('mp4-all-corrected','m4a',mp4(atom('\xa9nam',datum(Buffer.from('Title'))),atom('\xa9ART',datum(Buffer.from('Artist'))),atom('\xa9alb',datum(Buffer.from('Album'))),atom('\xa9gen',datum(Buffer.from('House'))),atom('\xa9day',datum(Buffer.from('2024-01-01'))),atom('tmpo',datum(Buffer.from([0,128]))),atom('covr',datum(image)),free('LABEL','Label')),{title:'Title',artist:'Artist',album:'Album',genre:'House',year:2024,bpm:128,label:'Label',artwork:[...image],artworkMimeType:'image/png'});
add('mp4-numeric-genre','m4a',mp4(atom('gnre',datum(Buffer.from([0,18])))));
add('empty-id3','mp3',tag([]));add('invalid-mp3','mp3',Buffer.from('not an mp3 file'));add('unsupported','wav',Buffer.from('not an audio file'));
const fixtures=new URL('../tests/fixtures/',import.meta.url);await mkdir(fixtures,{recursive:true});
const outputs=[];
for(const test of cases){const reads=[];const reader={size:test.bytes.length,extension:test.extension,read:async(offset,length)=>{reads.push([offset,length]);return test.bytes.subarray(offset,Math.min(offset+length,test.bytes.length));}};let expected=await api.extractMetadata(reader);if(expected?.artwork)expected={...expected,artwork:[...expected.artwork]};const fixture=new URL(`${test.name}.bin`,fixtures);if(process.argv.includes('--check'))assert.deepEqual(await readFile(fixture),test.bytes);else await writeFile(fixture,test.bytes);outputs.push({name:test.name,extension:test.extension,legacy:expected,expected:test.nativeExpected??expected,intentionalDifference:!!test.nativeExpected,reads});}
const utilities={bpm:['128','128.56 BPM','500','0','501','junk','x .7','12.3.4'].map(input=>({input,output:utils.parseBpm(input)??null})),year:['2024-01-01','1899','2100','2101','bad'].map(input=>({input,output:utils.parseYear(input)??null}))};
const manifest={source:'https://github.com/chrisle/metadata-connect',commit:'c2ae262650a7bbe7af5fb5222a6ab6b882ecf865',synthetic:true,originalTestCases:sourceTests.length,cases:outputs,utilities};
const manifestPath=new URL('oracle.json',fixtures),serialized=JSON.stringify(manifest,null,2)+'\n';if(process.argv.includes('--check'))assert.equal(await readFile(manifestPath,'utf8'),serialized);else await writeFile(manifestPath,serialized);console.log(`Original tests ${sourceTests.length} passed; metadata-connect extraction verified ${outputs.length} synthetic cases, including ${outputs.filter(x=>x.intentionalDifference).length} documented corrections`);
