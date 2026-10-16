// 仅检查已安装程序的公开路由代码，不访问会话存储。
import {openSync,readSync,closeSync} from 'node:fs';
const archive=process.argv[2];
const fd=openSync(archive,'r');
try {
 const prefix=Buffer.alloc(16);readSync(fd,prefix,0,16,0);
 const headerSize=prefix.readUInt32LE(4),jsonSize=prefix.readUInt32LE(12);
 const bytes=Buffer.alloc(jsonSize);readSync(fd,bytes,0,jsonSize,16);
 const header=JSON.parse(bytes.toString('utf8'));
 function* files(nodes,path=''){for(const [name,node]of Object.entries(nodes)){const file=path?path+'/'+name:name;if(node.files)yield*files(node.files,file);else if(node.offset)yield[file,node];}}
 let count=0;
 for(const [file,node]of files(header.files)){
  if(!file.endsWith('.js')||file.includes('node_modules/')||(process.argv[3]==='*'?false:process.argv[3]?!file.includes(process.argv[3]):!/(main|deep|protocol|router)/i.test(file)))continue;
  const data=Buffer.alloc(node.size);readSync(fd,data,0,node.size,8+headerSize+Number(node.offset));const text=data.toString('utf8');
  for(const term of process.argv[4]?[process.argv[4]]:['hostname===`threads`','case`threads`','hostname!==`threads`','codex://threads','/threads/','open-url','codex-deep-link']){
   const at=text.indexOf(term);if(at>=0){console.log(JSON.stringify({file,term,context:text.slice(Math.max(0,at-180),at+Number(process.argv[5]||300))}));if(++count>=12)process.exit(0);}
  }
 }
}finally{closeSync(fd);}
