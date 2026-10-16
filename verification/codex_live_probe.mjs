// 跟随当前聊天的现有 Desktop 实例；只记录事件类型和数量，不保存正文。
import net from 'node:net';
import { createHash, randomUUID } from 'node:crypto';
import { writeFileSync } from 'node:fs';

const threadId = process.env.CODEX_THREAD_ID;
if (!/^[a-f0-9-]{36}$/i.test(threadId || '')) throw new Error('缺少当前 CODEX_THREAD_ID');
const duration = Math.min(45, Math.max(5, Number(process.argv[2] || 25)));
const output = process.argv[3] || 'verification/codex-live-result.json';
const started = Date.now();
const report = {
  transport: 'existing-desktop-ipc',
  targetHash: createHash('sha256').update(threadId).digest('hex').slice(0, 12),
  connected: false, initialized: false, ownerFound: false,
  snapshots: 0, patchBatches: 0, patches: 0,
  eventTypes: {}, snapshotKeys: [], valueTypes: {}, statusValues: {},
  patchFields: {}, requestMethods: {},
  liveValueTypes: {}, liveStatusValues: {}, runtimeStates: [],
};
const pending = new Map();
let clientId = 'initializing-client';
let ownerId;
let buffer = Buffer.alloc(0);
let subscribed = false;
const socket = net.createConnection('\\\\.\\pipe\\codex-ipc');

function send(message) {
  const json = Buffer.from(JSON.stringify(message));
  const frame = Buffer.alloc(4 + json.length);
  frame.writeUInt32LE(json.length, 0);
  json.copy(frame, 4);
  socket.write(frame);
}
function request(method, params, version = 0, targetClientId) {
  if (!['initialize', 'thread-owner-discovery'].includes(method)) throw new Error('只读请求白名单拒绝');
  const requestId = randomUUID();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(requestId); reject(new Error(`${method}:timeout`)); }, 7000);
    pending.set(requestId, { resolve, reject, timer });
    send({ type: 'request', requestId, sourceClientId: clientId, version, method, params, targetClientId, timeoutMs: 6000 });
  });
}
function following(value) {
  send({ type: 'broadcast', method: 'thread-stream-following-changed', version: 1,
    sourceClientId: clientId, targetClientIds: [ownerId],
    params: { conversationId: threadId, hostId: 'local', following: value } });
}
function increment(target, key) { target[key] = (target[key] || 0) + 1; }
const statuses = new Set(['inProgress', 'in_progress', 'running', 'completed', 'failed', 'pending', 'waiting', 'interrupted', 'idle', 'active']);
function metadata(value, live = false) {
  if (!value || typeof value !== 'object') return;
  if (typeof value.type === 'string' && value.type.length < 70) increment(report.valueTypes, value.type);
  if (statuses.has(value.status)) increment(report.statusValues, value.status);
  if (live && typeof value.type === 'string' && value.type.length < 70) increment(report.liveValueTypes, value.type);
  if (live && statuses.has(value.status)) increment(report.liveStatusValues, value.status);
  if (typeof value.method === 'string' && /^item\/.+\/(requestApproval|requestUserInput)$|^mcpServer\/elicitation\/request$/.test(value.method)) increment(report.requestMethods, value.method);
  for (const child of Object.values(value)) if (child && typeof child === 'object') metadata(child, live);
}
function receive(message) {
  if (message.type === 'response') {
    const waiting = pending.get(message.requestId);
    if (waiting) {
      clearTimeout(waiting.timer); pending.delete(message.requestId); waiting.resolve(message);
    }
    return;
  }
  if (message.type === 'client-discovery-request') {
    send({ type: 'client-discovery-response', requestId: message.requestId, response: { canHandle: false } });
    return;
  }
  if (message.type !== 'broadcast') return;
  increment(report.eventTypes, message.method);
  const params = message.params;
  if (message.method !== 'thread-stream-state-changed' || params?.conversationId !== threadId) return;
  const change = params.change;
  if (change?.type === 'snapshot') {
    report.snapshots += 1;
    report.snapshotKeys = Object.keys(change.conversationState || {});
    const runtimeState = change.conversationState?.threadRuntimeStatus?.type;
    if (typeof runtimeState === 'string') report.runtimeStates.push(runtimeState);
    metadata(change.conversationState);
  } else if (change?.type === 'patches') {
    report.patchBatches += 1;
    report.patches += change.patches?.length || 0;
    for (const patch of change.patches || []) {
      const path = Array.isArray(patch.path) ? patch.path : String(patch.path || '').split('/');
      for (const part of path) if (typeof part === 'string' && /^[A-Za-z][A-Za-z0-9_]{0,45}$/.test(part)) increment(report.patchFields, part);
      metadata(patch.value, true);
      if (path.includes('threadRuntimeStatus') && typeof patch.value?.type === 'string') report.runtimeStates.push(patch.value.type);
    }
  }
}
socket.on('data', chunk => {
  buffer = Buffer.concat([buffer, chunk]);
  try {
    while (buffer.length >= 4) {
      const length = buffer.readUInt32LE(0);
      if (length <= 0 || length > 256 * 1024 * 1024) throw new Error('帧大小超出只读探针限制');
      if (buffer.length < length + 4) break;
      const frame = buffer.subarray(4, 4 + length);
      buffer = buffer.subarray(4 + length);
      receive(JSON.parse(frame.toString('utf8')));
    }
  } catch (error) { report.error = error.message; socket.destroy(); }
});
socket.on('error', error => { report.error = error.code || error.name; });

try {
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('connect:timeout')), 5000);
    socket.once('connect', () => { clearTimeout(timer); resolve(); });
    socket.once('error', error => { clearTimeout(timer); reject(error); });
  });
  report.connected = true;
  const initialized = await request('initialize', { clientType: 'agmanage-verification' });
  if (initialized.resultType !== 'success') throw new Error(`initialize:${initialized.error}`);
  clientId = initialized.result.clientId;
  report.initialized = true;
  const owner = await request('thread-owner-discovery', { conversationId: threadId, hostId: 'local' }, 1);
  if (owner.resultType !== 'success') throw new Error(`owner:${owner.error}`);
  ownerId = owner.handledByClientId;
  report.ownerFound = true;
  following(true);
  subscribed = true;
  process.stdout.write(JSON.stringify({ phase: 'following-current-desktop-session', ownerFound: true }) + '\n');
  await new Promise(resolve => setTimeout(resolve, duration * 1000));
} catch (error) {
  report.error = error.code || error.message;
} finally {
  if (subscribed && socket.writable) following(false);
  socket.destroy();
  for (const waiting of pending.values()) { clearTimeout(waiting.timer); waiting.reject(new Error('probe-stopped')); }
  pending.clear();
  report.elapsedMs = Date.now() - started;
  writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  process.stdout.write(JSON.stringify(report) + '\n');
}
