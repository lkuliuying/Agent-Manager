import net from 'node:net';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { homedir } from 'node:os';
import { createHash, randomUUID } from 'node:crypto';

export const PROTOCOL = 1;
export const MAX_FRAME = 32 * 1024 * 1024;
export const identity = value => createHash('sha256').update(String(value).toLowerCase()).digest('hex').slice(0, 24);
// 与管理器共用用户目录，避免打包应用将 AppData 映射到各自的私有缓存。
export const managerHome = () => join(process.env.USERPROFILE || homedir(), '.agmanage');
export const text = value => typeof value === 'string' ? value : '';
export const time = value => typeof value === 'number' ? value : (Date.parse(value) || 0);
// 仅记录插件加载阶段和固定错误码，用于诊断宿主未连接；不含会话或环境变量。
const healthWrites = new Map();
export function reportHealth(kind, stage, code = '') {
  const folder = managerHome();
  const prior = healthWrites.get(kind);
  const key = `${stage}:${code}`;
  if (prior?.key === key) return prior.write;
  const write = (prior?.write || Promise.resolve()).then(() => mkdir(folder, { recursive: true })).then(() => writeFile(join(folder, `observer-health-${kind}.json`), JSON.stringify({ stage, code, pid: process.pid, updatedAt: Date.now() }))).catch(() => {});
  healthWrites.set(kind, { key, write });
  return write;
}
export function frame(value) {
  const data = Buffer.from(JSON.stringify(value));
  if (!data.length || data.length > MAX_FRAME) throw new Error('frame_size');
  const header = Buffer.alloc(4); header.writeUInt32LE(data.length);
  return Buffer.concat([header, data]);
}

export async function hostVersion(names) {
  for (const argument of process.argv.slice(0, 3)) {
    let folder = dirname(argument);
    for (let i = 0; i < 12; i++) {
      try {
        const pkg = JSON.parse(await readFile(join(folder, 'package.json'), 'utf8'));
        if (names.includes(pkg.name)) return pkg.version;
      } catch { /* 仅尝试宿主入口的父目录，不扫描用户文件。 */ }
      const parent = dirname(folder); if (parent === folder) break; folder = parent;
    }
  }
  return 'unknown';
}

// 宿主事件只标记失效；后台串行读取权威快照，避免 delta 与快照交错导致重复正文。
export function createObserver({ kind, instance, version, list, snapshot, descriptor, interval = 200, report = () => {} }) {
  const connection = randomUUID();
  const subscribed = new Set();
  const dirty = new Set();
  let socket, buffer = Buffer.alloc(0), stopped = false, ready = false;
  let listDirty = true, busy = false, seq = 0, lastList = 0, lastBeat = 0, nextConnect = 0;
  let active = new AbortController();
  const bounded = (operation, signal) => {
    const timeout = AbortSignal.timeout(12000);
    const combined = AbortSignal.any([signal, timeout]);
    return Promise.race([operation(combined), new Promise((_, reject) => combined.addEventListener('abort', () => reject(new Error('read_aborted')), { once: true }))]);
  };
  const send = value => {
    if (!socket || !ready || socket.destroyed) return false;
    try {
      const bytes = frame({ ...value, seq: ++seq });
      if (socket.writableLength + bytes.length > MAX_FRAME) { socket.destroy(); return false; }
      socket.write(bytes); return true;
    } catch { socket.destroy(); return false; }
  };
  const failure = (code, id) => send({ type: 'error', code, ...(id ? { id } : {}) });
  async function receive(request) {
    if (request.protocol !== PROTOCOL) throw new Error('protocol');
    switch (request.type) {
      case 'catalog': listDirty = true; break;
      case 'subscribe':
      case 'snapshot':
        if (typeof request.id !== 'string' || !request.id) throw new Error('identity');
        subscribed.add(request.id); dirty.add(request.id); break;
      case 'unsubscribe': subscribed.delete(request.id); dirty.delete(request.id); break;
      default: throw new Error('readonly_violation');
    }
  }
  async function connect() {
    nextConnect = Date.now() + 2000;
    try {
      const address = JSON.parse(await readFile(descriptor || join(managerHome(), 'observer.json'), 'utf8'));
      if (address.protocol !== PROTOCOL || !/^\\\\\.\\pipe\\agmanage-observer-[a-z0-9-]+$/.test(address.pipe)) return;
      socket = net.createConnection(address.pipe);
      socket.unref();
      socket.on('error', error => { report('connect_failed', ['EACCES', 'ENOENT', 'ECONNREFUSED'].includes(error.code) ? error.code : 'pipe_error'); });
      socket.on('connect', () => {
        ready = true; buffer = Buffer.alloc(0); seq = 0; listDirty = true;
        report('connected', version);
        send({ type: 'hello', protocol: PROTOCOL, kind, instance, connection, version, pluginVersion: '0.2.0' });
      });
      socket.on('close', () => {
        ready = false; socket = undefined; subscribed.clear(); dirty.clear();
        active.abort(); active = new AbortController();
      });
      socket.on('data', chunk => {
        try {
          buffer = Buffer.concat([buffer, chunk]);
          while (buffer.length >= 4) {
            const size = buffer.readUInt32LE();
            if (!size || size > MAX_FRAME) throw new Error('frame_size');
            if (buffer.length < size + 4) break;
            const request = JSON.parse(buffer.subarray(4, size + 4));
            buffer = buffer.subarray(size + 4);
            receive(request).catch(() => socket?.destroy());
          }
        } catch { socket?.destroy(); }
      });
    } catch { /* 管理器未启动时保持宿主正常运行，等待下次本地连接。 */ }
  }
  async function tick() {
    if (stopped || busy) return;
    if (!socket) { busy = true; try { if (Date.now() >= nextConnect) await connect(); } finally { busy = false; } return; }
    if (!ready) return;
    if (Date.now() - lastBeat > 4000) { send({ type: 'heartbeat' }); lastBeat = Date.now(); }
    busy = true;
    const signal = active.signal;
    try {
      if (listDirty || Date.now() - lastList > 20000) {
        listDirty = false; lastList = Date.now();
        try {
          const sessions = await bounded(readSignal => list(readSignal), signal);
          if (!signal.aborted) send({ type: 'catalog', sessions });
        } catch { if (!signal.aborted) failure('catalog_read_failed'); }
      }
      // 每次读取单个会话；读取期间新到事件保留在 dirty 中，下一轮再次获取。
      const id = dirty.values().next().value;
      if (id && subscribed.has(id)) {
        dirty.delete(id);
        try {
          const session = await bounded(readSignal => snapshot(id, readSignal), signal);
          if (!signal.aborted && subscribed.has(id)) send({ type: 'snapshot', session });
        } catch { if (!signal.aborted) failure('snapshot_read_failed', id); }
      }
    } finally { busy = false; }
  }
  const timer = setInterval(() => { tick().catch(() => failure('observer_failed')); }, interval);
  timer.unref();
  return {
    changed(id) { if (subscribed.has(id)) dirty.add(id); listDirty = true; },
    contentChanged(id) { if (subscribed.has(id)) dirty.add(id); },
    catalogChanged() { listDirty = true; },
    close() { stopped = true; clearInterval(timer); active.abort(); socket?.destroy(); subscribed.clear(); dirty.clear(); },
  };
}
