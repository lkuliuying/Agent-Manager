// 复用已验证的只读订阅实现，只保存字段结构和状态枚举，正文及标识符不落盘。
import { readFileSync } from 'node:fs';
const source = readFileSync(new URL('./codex_multi_probe.mjs', import.meta.url), 'utf8');
const prefix = source.slice(0, source.indexOf('\nfunction startPhase('));
const inspect = `
function shape(value, depth = 0, key = '') {
  if (depth > 9) return typeof value;
  if (typeof value === 'string') return ['type','role','status'].includes(key) && /^[a-zA-Z_]+$/.test(value) ? value : 'string';
  if (Array.isArray(value)) return {length:value.length, last:value.slice(-3).map(v=>shape(v,depth+1,key))};
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k,v],i)=>[/[0-9a-f]{8}-[0-9a-f-]{27}/i.test(k)||k.includes(':')?'entry-'+i:k,shape(v,depth+1,k)]));
  return value === null ? null : typeof value;
}
function startPhase(session) { session.report.phases.push({snapshots:0,patchBatches:0,patches:0,textChanges:0,liveSnapshotTextChanges:0,textKindsChanged:{},acceptedChangeTypes:{},liveItemStatuses:{},runtimeTransitions:[]}); session.followStarted=Date.now(); }
try {
 const discovery = JSON.parse(execFileSync('python',[fileURLToPath(new URL('./discover_desktop_threads.py',import.meta.url))],{encoding:'utf8',windowsHide:true,stdio:['ignore','pipe','pipe']}));
 current=new MonitorConnection(); await current.initialize();
 for (const id of discovery.candidates) {
  const owner=await current.owner(id); if(!owner) continue;
  const session={id,owner,report:{phases:[]}}; sessions.set('local:'+id,session); startPhase(session); current.follow(session,true); if(sessions.size===3)break;
 }
 await delay(3500);
 const schema=[...sessions.values()].filter(s=>s.state).map(s=>{
  const state=s.state;
  const history=state.turnHistory?.history;
  const turns=[...(state.turns||[]),...Object.values(history?.entitiesByKey||{})];
  const kinds={};
  for(const turn of turns)for(const item of turn.items||[])if(/^[a-zA-Z]+$/.test(item.type||''))kinds[item.type]=shape(item);
  const commandOutcomes={};for(const turn of turns)for(const item of turn.items||[])if(item.type==='commandExecution'){const outcome=String(item.status)+':'+String(item.exitCode);commandOutcomes[outcome]=(commandOutcomes[outcome]||0)+1;}
  return {rootFields:Object.keys(state),runtime:shape(state.threadRuntimeStatus),historyKind:/^[a-zA-Z]+$/.test(state.turnHistory?.kind||'')?state.turnHistory.kind:'unknown',historyFields:Object.keys(history||{}),turnFields:[...new Set(turns.flatMap(t=>Object.keys(t)))],turnStatuses:[...new Set(turns.map(t=>t.status).filter(v=>/^[a-zA-Z]+$/.test(v||'')))],commandOutcomes,itemKinds:kinds,islands:shape(history?.islands)};
 });
 writeFileSync('verification/native-stream-schema.json',JSON.stringify(schema,null,2)+'\\n');
 console.log(JSON.stringify({snapshots:schema.length,output:'verification/native-stream-schema.json',rawContentSaved:false}));
} finally { if(current?.socket.writable)for(const s of sessions.values())current.follow(s,false); await delay(100); current?.close(); }
`;
const discovery = `
import { DatabaseSync } from 'node:sqlite';
import { readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
function execFileSync() {
 const root=process.env.CODEX_HOME||join(homedir(),'.codex');
 const files=readdirSync(root).filter(f=>/^state_\\d+\\.sqlite$/.test(f)).sort((a,b)=>Number(b.match(/\\d+/)[0])-Number(a.match(/\\d+/)[0]));
 const db=new DatabaseSync(join(root,files[0]),{readOnly:true});
 try { return JSON.stringify({candidates:db.prepare("SELECT id FROM threads WHERE source='vscode' AND archived=0 ORDER BY updated_at DESC LIMIT 1000").all().map(v=>v.id)}); }
 finally {db.close();}
}
`;
try {
 await import('data:text/javascript;base64,' + Buffer.from(prefix.replace("import { execFileSync } from 'node:child_process';",discovery) + inspect.replace("new URL('./discover_desktop_threads.py',import.meta.url)","new URL('./discover_desktop_threads.py','file:///F:/Program/agmanage/verification/')")).toString('base64'));
} catch(error) {
 console.error(JSON.stringify({error:error.code||'schema_inspection_failed',rawContentSaved:false}));
 process.exitCode=1;
}
