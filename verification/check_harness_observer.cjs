// 用桌面自带 Cordis 和合成会话检查插件服务边界，不启动桌面运行时或读取真实会话。
const assert = require('node:assert/strict');
const net = require('node:net');
const { mkdtemp, mkdir, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { pathToFileURL } = require('node:url');
const { randomUUID } = require('node:crypto');
const { setTimeout: delay } = require('node:timers/promises');

(async () => {
  const folder = await mkdtemp(join(tmpdir(), 'agmanage-cordis-test-'));
  const previousHome = process.env.USERPROFILE;
  process.env.USERPROFILE = folder;
  const { Context } = await import(pathToFileURL(join(process.argv[2], 'node_modules/@deepseek-ai/cordis/lib/index.js')));
  const { apply } = await import(pathToFileURL(join(__dirname, '../plugins/deepseek/index.mjs')));
  const { frame, reportHealth } = await import(pathToFileURL(join(__dirname, '../plugins/common/observer.mjs')));
  const pipe = `\\\\.\\pipe\\agmanage-observer-${randomUUID()}`;
  const peers = new Set();
  const packets = [];
  const server = net.createServer(socket => {
    peers.add(socket);
    let buffer = Buffer.alloc(0);
    socket.on('error', () => {});
    socket.on('close', () => peers.delete(socket));
    socket.on('data', data => {
      buffer = Buffer.concat([buffer, data]);
      while (buffer.length >= 4 && buffer.length >= buffer.readUInt32LE() + 4) {
        const size = buffer.readUInt32LE();
        const packet = JSON.parse(buffer.subarray(4, size + 4));
        buffer = buffer.subarray(size + 4);
        packets.push(packet);
        if (packet.type === 'hello') socket.write(frame({ protocol: 1, type: 'subscribe', id: 'fixture-session' }));
      }
    });
  });
  let ctx;
  try {
    await new Promise(resolve => server.listen(pipe, resolve));
    await mkdir(join(folder, '.agmanage'), { recursive: true });
    await writeFile(join(folder, '.agmanage/observer.json'), JSON.stringify({ protocol: 1, pipe }));
    for (const withAgents of [true, false]) {
      packets.length = 0;
      ctx = new Context();
      ctx.provide('profileContext', { name: 'desktop' });
      const header = { id: 'fixture-session', cwd: 'C:/fixture', createdAt: 100 };
      // 服务由兄弟插件提供，才能捕获未声明 inject 的属性访问。
      await ctx.plugin(provider => {
        provider.provide('sessions', { get: () => ({ id: header.id }) });
        provider.provide('sessionQuery', {
          listSessions: async () => [{ header, live: true }],
          readTitleSnapshots: async () => [{ sessionId: header.id, status: 'fulfilled', value: { title: { title: '合成会话' } } }],
          readSession: async () => ({ session: header, events: [{ seq: 0, type: 'user/message', time: 100, data: { content: [{ type: 'text', text: '合成正文' }] } }] }),
        });
        if (withAgents) provider.provide('agents', { get: () => ({ status: 'running' }) });
      });
      await ctx.plugin(apply);
      const deadline = Date.now() + 5000;
      while (!packets.some(p => p.type === 'catalog') && !packets.some(p => p.type === 'error') && Date.now() < deadline) await delay(20);
      assert.deepEqual(packets.filter(p => p.type === 'error'), [], '目录读取不能因未注入可选服务而失败');
      const catalog = packets.find(p => p.type === 'catalog');
      assert.equal(catalog?.sessions[0]?.title, '合成会话');
      assert.equal(catalog.sessions[0].status, withAgents ? 'running' : 'unknown');
      while (!packets.some(p => p.type === 'snapshot') && Date.now() < deadline) await delay(20);
      assert.equal(packets.find(p => p.type === 'snapshot')?.session.messages[0]?.text, '合成正文');
      await ctx.fiber.dispose();
      ctx = undefined;
      await delay(30);
    }
    console.log(JSON.stringify({ passed: true, cases: 2, realCordis: true, syntheticSessions: true }));
  } finally {
    await ctx?.fiber.dispose();
    for (const socket of peers) socket.destroy();
    await new Promise(resolve => server.close(resolve));
    await reportHealth('deepseek', 'test_finished');
    if (previousHome === undefined) delete process.env.USERPROFILE; else process.env.USERPROFILE = previousHome;
    await rm(folder, { recursive: true, force: true });
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
