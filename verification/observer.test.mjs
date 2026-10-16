import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { createObserver, frame, reportHealth } from '../plugins/common/observer.mjs';
import { createSource, taskStatus } from '../plugins/opencode/source.mjs';
// 合成插件诊断也放在独立目录，避免覆盖真实桌面插件的状态。
const testHome = await mkdtemp(join(tmpdir(), 'agmanage-plugin-test-'));
const priorHome = process.env.USERPROFILE;
process.env.USERPROFILE = testHome;
const { projectHistory, acceptFrame, apply } = await import('../plugins/deepseek/index.mjs');
test.after(async () => {
  await reportHealth('deepseek', 'test_finished');
  if (priorHome === undefined) delete process.env.USERPROFILE; else process.env.USERPROFILE = priorHome;
  await rm(testHome, { recursive: true, force: true });
});

test('OpenCode 仅调用读取方法，使用 v1 路径契约并按片段保留消息', async () => {
  const reads=[];
  const message={info:{id:'m',role:'assistant',time:{created:100,completed:200},finish:'stop'},parts:[{id:'p',type:'text',text:'中文输出'},{id:'tool',type:'tool',tool:'read_file',state:{status:'completed',input:{path:'fixture'},output:'演示'}}]};
  const read=(name,value)=>async options=>{reads.push(name);assert.equal(options.query.directory,'C:/demo');if(name==='get'||name==='messages')assert.equal(options.path.id,'same');return {data:value};};
  const source=createSource({directory:'C:/demo',client:{session:{list:read('list',[{id:'same',directory:'C:/demo',title:'演示'}]),status:read('status',{}),get:read('get',{id:'same',directory:'C:/demo',title:'演示'}),messages:read('messages',[message])}}});
  assert.equal((await source.list())[0].id,'same');
  const snapshot=await source.snapshot('same');assert.equal(snapshot.messages[0].text,'中文输出');assert.equal(snapshot.status,'completed');assert.equal(snapshot.messages[1].kind,'read_file');
  assert.ok(reads.every(name=>['list','status','get','messages'].includes(name)));
  assert.equal(taskStatus({type:'idle'},[{info:{role:'user'}}]),'idle');
  assert.equal(taskStatus({type:'retry'},[message]),'running');
  assert.equal(taskStatus({type:'idle'},[{info:{role:'assistant',error:{name:'MessageAbortedError'}}}]),'interrupted');
});

test('Harness 历史投影区分完成、中断和未加载的未结束轮次',()=>{
  const record={session:{id:'same',cwd:'C:/demo',createdAt:10},events:[{seq:0,time:10,type:'user/message',data:{content:[{type:'text',text:'请求'},{type:'reasoning',text:'不展示内部推理'}]}},{seq:1,time:11,type:'turn/start',data:{}},{seq:2,time:12,type:'assistant/message',data:{message:{content:[{type:'text',text:'回复'}]}}}]};
  assert.equal(projectHistory(record,false).status,'unknown');
  assert.equal(projectHistory(record,true).status,'running');
  record.events.push({seq:3,time:13,type:'turn/end',data:{reason:{kind:'interrupted'}}});
  const snapshot=projectHistory(record,false);assert.equal(snapshot.status,'interrupted');assert.equal(snapshot.messages.length,2);assert.equal(snapshot.messages[0].text,'请求');
});

test('Harness 流式缺口不会伪造完整正文，最终块替换 delta',()=>{
  let state=acceptFrame(undefined,{type:'start',attemptId:'a',revision:1});
  state=acceptFrame(state,{type:'chunk',attemptId:'a',revision:2,index:0,chunk:{type:'text-delta',index:0,text:'你'}});
  state=acceptFrame(state,{type:'chunk',attemptId:'a',revision:3,index:1,chunk:{type:'block-end',index:0,block:{type:'text',text:'你好'}}});
  assert.equal(state.text,'你好');
  const gap=acceptFrame(state,{type:'chunk',attemptId:'a',revision:5,index:3,chunk:{type:'text-delta',index:0,text:'重复'}});
  assert.equal(gap.partial,true);assert.equal(gap.active,false);
  assert.equal(acceptFrame(state,{type:'end',attemptId:'a',revision:4}).text,'');
});

test('Harness 插件只注册观察回调，不注册审批或调用 follow',()=>{
  const events=[];let cleanup;
  const context={profileContext:{name:'desktop'},sessionQuery:{},sessions:{},on(name,handler){events.push([name,handler]);},effect(factory){cleanup=factory();},inject(keys,fn){assert.deepEqual(keys,['sessionQuery','sessions']);fn(this);},sessionController:{follow(){throw new Error('禁止激活');}}};
  apply(context);
  assert.deepEqual(events.map(([name])=>name),['session/created','session/disposed','session/event','agent/assistant-stream']);
  for(const [,callback] of events) assert.doesNotThrow(()=>callback(undefined));
  cleanup();
});

test('命名管道快照读取期间更新、重连与控制拒绝', {timeout:15000}, async()=>{
  const folder=await mkdtemp(join(tmpdir(),'agmanage-observer-test-'));
  const pipe=`\\\\.\\pipe\\agmanage-observer-${randomUUID()}`;
  const descriptor=join(folder,'observer.json');
  await writeFile(descriptor,JSON.stringify({protocol:1,pipe}));
  let socket;let buffer=Buffer.alloc(0);const packets=[];
  const server=net.createServer(client=>{socket=client;client.on('error',()=>{});client.on('data',chunk=>{buffer=Buffer.concat([buffer,chunk]);while(buffer.length>=4&&buffer.length>=buffer.readUInt32LE()+4){const n=buffer.readUInt32LE();packets.push(JSON.parse(buffer.subarray(4,n+4)));buffer=buffer.subarray(n+4);}});});
  await new Promise(resolve=>server.listen(pipe,resolve));
  let observer,release,reads=0;
  observer=createObserver({kind:'opencode',instance:'fixture',version:'1.18.27',descriptor,interval:15,list:async()=>[{id:'s'}],snapshot:async()=>{reads++;if(reads===1)await new Promise(resolve=>release=resolve);return {id:'s',text:`正文 ${reads}`};}});
  const until=async fn=>{const deadline=Date.now()+6000;while(!fn()){if(Date.now()>deadline)throw new Error('等待超时');await delay(15);}};
  try{
    await until(()=>packets.some(p=>p.type==='hello'));
    socket.write(frame({protocol:1,type:'subscribe',id:'s'}));
    await until(()=>release);observer.contentChanged('s');release();
    await until(()=>packets.filter(p=>p.type==='snapshot').length===2);
    const snapshots=packets.filter(p=>p.type==='snapshot');assert.equal(snapshots[1].session.text,'正文 2');assert.ok(snapshots[1].seq>snapshots[0].seq);
    const closed=new Promise(resolve=>socket.once('close',resolve));socket.write(frame({protocol:1,type:'send-message',id:'s'}));await closed;
    await until(()=>packets.filter(p=>p.type==='hello').length===2);
    assert.equal(reads,2);
  }finally{observer.close();socket?.destroy();await new Promise(resolve=>server.close(resolve));await rm(folder,{recursive:true,force:true});}
});
