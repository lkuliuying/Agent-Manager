import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import assert from 'node:assert/strict';
import { createObserver } from '../plugins/common/observer.mjs';

const executable=resolve(process.argv[2] || 'target/x86_64-pc-windows-msvc/debug/agmanage.exe');
// 独立通道描述文件，避免真实桌面插件混入合成验收。
const fixtureHome=await mkdtemp(join(tmpdir(),'agmanage-pipe-fixture-'));
const originalProfile=process.env.USERPROFILE;
process.env.USERPROFILE=fixtureHome;
const child=spawn(executable,['--probe-observers','--seconds','7'],{windowsHide:true,env:{...process.env,USERPROFILE:fixtureHome},stdio:['ignore','pipe','pipe']});
let output='',error='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>error+=b);
const exit=new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',code=>resolve(code));});
const observers=[];
try {
  await delay(300);
  const descriptor=join(fixtureHome,'.agmanage','observer.json');
  const address=JSON.parse(await readFile(descriptor,'utf8'));assert.equal(address.protocol,1);
  for (const [kind,version] of [['opencode','1.18.27'],['deepseek','0.2.0-rc.2']]) {
    const metadata={id:'same-native-id',title:'合成会话',project:'synthetic',created:100,status:'running',historyComplete:false};
    observers.push(createObserver({kind,version,instance:'synthetic-fixture',interval:30,list:async()=>[metadata],snapshot:async()=>({...metadata,historyComplete:true,messages:[{id:'m',author:'Agent',kind:'message',status:'completed',time:100,text:'仅用于通道验证的合成消息'}]})}));
  }
  assert.equal(await exit,0,error);
  const result=JSON.parse(output.trim());
  assert.equal(result.errors.length,0);
  for (const source of result.sources) {assert.equal(source.connections,1);assert.equal(source.sessions,1);assert.equal(source.snapshots,1);assert.equal(source.messages,1);}
  assert.equal(result.controlsSent,0);
  await mkdir('artifacts/observer',{recursive:true});
  await writeFile('artifacts/observer/fixture-result.json',JSON.stringify({...result,passed:true,syntheticOnly:true},null,2));
  console.log(JSON.stringify({passed:true,sources:2,sameNativeIdIsolated:true,syntheticOnly:true}));
} finally {for(const observer of observers)observer.close();if(child.exitCode===null){child.kill();await exit;}if(originalProfile===undefined)delete process.env.USERPROFILE;else process.env.USERPROFILE=originalProfile;await rm(fixtureHome,{recursive:true,force:true});}
