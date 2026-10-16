// 仅跟随已有 Desktop 会话。每个会话独立应用增量；断线只关闭本验证器的管道。
import net from 'node:net';
import { createHash, randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const seconds = Math.min(45, Math.max(8, Number(process.argv[2] || 18)));
const output = process.argv[3] || 'verification/codex-multi-result.json';
const waitForContent = process.argv[4] === 'wait-for-content';
const waitSeconds = Math.min(1800, Math.max(60, Number(process.argv[5] || 300)));
const hash = value => createHash('sha256').update(value).digest('hex').slice(0, 16);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const started = Date.now();
const report = {
  startedAt: new Date().toISOString(), transport: 'existing-desktop-ipc',
  discovery: { method: 'readonly-thread-index-plus-existing-owner-discovery' },
  readonly: { databaseMode: 'ro', requests: {}, broadcasts: {}, controlsSent: 0,
    agentProcessesStarted: 0, threadResumeCalls: 0, threadCreateCalls: 0 },
  reconnect: { forcedMonitorDisconnects: 0, attempts: 0, succeeded: false, offlineMs: 0 },
  isolation: { checkedBatches: 0, crossSessionMutations: 0, identityMismatches: 0,
    unexpectedSessionEvents: 0, wrongOwnerEvents: 0, revisionGaps: 0,
    patchFailures: 0, lateGenerationEvents: 0 },
  sessions: [], errors: [],
};
const sessions = new Map();
let generation = 0;
let current;
function count(map, key) { map[key] = (map[key] || 0) + 1; }
function code(error) { return /^[A-Za-z0-9_:-]{1,100}$/.test(error?.code || '') ? error.code : 'probe_error'; }
function fail(value) { const error = new Error(value); error.code = value; throw error; }
const runtimeEnums = new Set(['active', 'idle', 'notLoaded', 'systemError', 'unknown']);
const statusEnums = new Set(['inProgress', 'in_progress', 'running', 'completed', 'failed',
  'pending', 'waiting', 'interrupted', 'idle', 'active', 'cancelled', 'error']);
function runtime(state) {
  const value = state?.threadRuntimeStatus?.type;
  return runtimeEnums.has(value) ? value : 'unknown';
}
function textSummary(value, prefix = [], result = new Map()) {
  if (typeof value === 'string') {
    const field = prefix.at(-1);
    const parent = prefix.at(-2);
    const kind = field === 'text' ? 'message' : field === 'aggregatedOutput' ? 'toolOutput'
      : ['summary', 'content'].includes(parent) && /^\d+$/.test(String(field)) ? 'reasoning' : null;
    if (kind) result.set(JSON.stringify(prefix), { digest: hash(value), kind });
  } else if (value && typeof value === 'object') {
    for (const [key, child] of Object.entries(value)) textSummary(child, [...prefix, key], result);
  }
  return result;
}
function statusSummary(value, target) {
  if (!value || typeof value !== 'object') return;
  if (statusEnums.has(value.status)) count(target, value.status);
  for (const child of Object.values(value)) {
    if (child && typeof child === 'object') statusSummary(child, target);
  }
}
function recordTextChanges(before, after, phase, source = 'patch') {
  for (const [key, text] of after) {
    if (before.get(key)?.digest !== text.digest) {
      phase.textChanges++; count(phase.textKindsChanged, text.kind);
      if (source === 'snapshot') phase.liveSnapshotTextChanges++;
    }
  }
}
function getAt(root, path) {
  let value = root;
  for (const part of path) value = value?.[part];
  return value;
}
function patchState(root, patch) {
  const path = patch.path;
  if (!Array.isArray(path) || !['add', 'replace', 'remove'].includes(patch.op)) fail('invalid_patch');
  if (!path.length) {
    if (patch.op === 'remove') fail('root_remove');
    return patch.value;
  }
  const parent = getAt(root, path.slice(0, -1));
  const key = path.at(-1);
  if (!parent || typeof parent !== 'object' || ['__proto__', 'prototype', 'constructor'].includes(key)) fail('invalid_patch_parent');
  if (Array.isArray(parent)) {
    if (key === 'length' && patch.op === 'replace' && Number.isInteger(patch.value)) parent.length = patch.value;
    else if (!Number.isInteger(Number(key)) || Number(key) < 0 || Number(key) > parent.length) fail('invalid_array_index');
    else if (patch.op === 'remove') parent.splice(Number(key), 1);
    else if (patch.op === 'add') parent.splice(Number(key), 0, patch.value);
    else parent[Number(key)] = patch.value;
  } else if (patch.op === 'remove') delete parent[key];
  else parent[key] = patch.value;
  return root;
}

class MonitorConnection {
  constructor() {
    this.generation = ++generation;
    this.clientId = 'initializing-client';
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.socket = net.createConnection('\\\\.\\pipe\\codex-ipc');
    this.socket.on('data', chunk => this.data(chunk));
    this.socket.on('error', error => { this.lastError = code(error); });
    this.socket.on('close', () => {
      for (const waiting of this.pending.values()) {
        clearTimeout(waiting.timer); waiting.reject(Object.assign(new Error(), { code: 'connection_closed' }));
      }
      this.pending.clear();
    });
  }
  send(message) {
    if (!this.socket.writable) fail('connection_not_writable');
    const data = Buffer.from(JSON.stringify(message));
    const frame = Buffer.alloc(data.length + 4);
    frame.writeUInt32LE(data.length, 0); data.copy(frame, 4);
    this.socket.write(frame);
  }
  request(method, params, version = 0) {
    if (!['initialize', 'thread-owner-discovery'].includes(method)) fail('readonly_request_rejected');
    count(report.readonly.requests, method);
    const requestId = randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        reject(Object.assign(new Error(), { code: 'request_timeout' }));
      }, 7000);
      this.pending.set(requestId, { resolve, reject, timer });
      this.send({ type: 'request', requestId, sourceClientId: this.clientId,
        version, method, params, timeoutMs: 6000 });
    });
  }
  async initialize() {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Object.assign(new Error(), { code: 'connect_timeout' })), 5000);
      this.socket.once('connect', () => { clearTimeout(timer); resolve(); });
      this.socket.once('error', error => { clearTimeout(timer); reject(error); });
    });
    const response = await this.request('initialize', { clientType: 'agmanage-readonly-multi-verification' });
    if (response.resultType !== 'success' || !response.result?.clientId) fail('initialize_failed');
    this.clientId = response.result.clientId;
  }
  async owner(id) {
    const response = await this.request('thread-owner-discovery', { conversationId: id, hostId: 'local' }, 1);
    return response.resultType === 'success' ? response.handledByClientId : null;
  }
  follow(session, following) {
    count(report.readonly.broadcasts, 'thread-stream-following-changed');
    this.send({ type: 'broadcast', method: 'thread-stream-following-changed', version: 1,
      sourceClientId: this.clientId, targetClientIds: [session.owner],
      params: { conversationId: session.id, hostId: 'local', following } });
  }
  data(chunk) {
    try {
      this.buffer = Buffer.concat([this.buffer, chunk]);
      while (this.buffer.length >= 4) {
        const length = this.buffer.readUInt32LE(0);
        if (length <= 0 || length > 256 * 1024 * 1024) fail('invalid_frame_size');
        if (this.buffer.length < length + 4) break;
        const frame = this.buffer.subarray(4, length + 4);
        this.buffer = this.buffer.subarray(length + 4);
        this.receive(JSON.parse(frame.toString('utf8')));
      }
    } catch (error) { report.errors.push(code(error)); this.socket.destroy(); }
  }
  receive(message) {
    if (this.generation !== generation) { report.isolation.lateGenerationEvents++; return; }
    if (message.type === 'response') {
      const waiting = this.pending.get(message.requestId);
      if (waiting) { clearTimeout(waiting.timer); this.pending.delete(message.requestId); waiting.resolve(message); }
      return;
    }
    if (message.type === 'client-discovery-request') {
      // 不声明能够接管任何会话，也不接收执行或审批请求。
      this.send({ type: 'client-discovery-response', requestId: message.requestId, response: { canHandle: false } });
      return;
    }
    if (message.type !== 'broadcast' || message.method !== 'thread-stream-state-changed') return;
    const params = message.params;
    const session = sessions.get(`${params?.hostId}:${params?.conversationId}`);
    if (!session) { report.isolation.unexpectedSessionEvents++; return; }
    if (message.sourceClientId !== session.owner) { report.isolation.wrongOwnerEvents++; return; }
    const phase = session.report.phases.at(-1);
    const change = params.change;
    if (change?.type === 'snapshot') {
      const state = change.conversationState;
      if (state?.id !== session.id || (state.hostId && state.hostId !== 'local')) {
        report.isolation.identityMismatches++; return;
      }
      if (session.previousOwner === session.owner && session.previousRevision > change.revision) {
        report.errors.push('snapshot_revision_regressed'); return;
      }
      if (session.syncedGeneration === generation) {
        recordTextChanges(textSummary(session.state), textSummary(state), phase, 'snapshot');
        if (runtime(session.state) !== runtime(state)) {
          phase.runtimeTransitions.push({ from: runtime(session.state), to: runtime(state) });
        }
      }
      session.state = state; session.revision = change.revision; session.syncedGeneration = generation;
      phase.snapshots++;
      if (phase.snapshots === 1) {
        phase.snapshotRevision = change.revision;
        phase.snapshotWaitMs = Date.now() - session.followStarted;
        phase.initialRuntime = runtime(state);
        phase.initialTextFields = textSummary(state).size;
        phase.initialItemStatuses = {}; statusSummary(state, phase.initialItemStatuses);
      }
      if (session.offlineDigest) {
        phase.changedWhileDisconnected = hash(JSON.stringify(state)) !== session.offlineDigest;
        phase.revisionAdvancedWhileDisconnected = change.revision > session.previousRevision;
      }
      return;
    }
    if (change?.type !== 'patches') return;
    if (session.syncedGeneration !== generation || change.baseRevision !== session.revision) {
      report.isolation.revisionGaps++; return;
    }
    const others = [...sessions.values()].filter(other => other !== session)
      .map(other => [other, other.state, other.revision]);
    const previousRuntime = runtime(session.state);
    try {
      for (const patch of change.patches || []) {
        const previous = getAt(session.state, patch.path || []);
        const beforeText = textSummary(previous, patch.path || []);
        session.state = patchState(session.state, patch);
        const next = getAt(session.state, patch.path || []);
        const afterText = textSummary(next, patch.path || []);
        recordTextChanges(beforeText, afterText, phase);
        if (patch.path?.at(-1) === 'status' && statusEnums.has(patch.value)) count(phase.liveItemStatuses, patch.value);
        statusSummary(patch.value, phase.liveItemStatuses);
      }
      for (const accepted of change.acceptedTextChanges || []) {
        if (accepted.key?.threadId !== session.id || accepted.key?.hostId !== 'local') {
          report.isolation.identityMismatches++;
        }
        if (['text', 'text-parts'].includes(accepted.type)) count(phase.acceptedChangeTypes, accepted.type);
      }
      session.revision = change.revision;
      phase.patchBatches++; phase.patches += change.patches?.length || 0;
      phase.lastRevision = change.revision;
      const nextRuntime = runtime(session.state);
      if (previousRuntime !== nextRuntime) phase.runtimeTransitions.push({ from: previousRuntime, to: nextRuntime });
      if (session.state.id !== session.id) report.isolation.identityMismatches++;
      report.isolation.checkedBatches++;
      for (const [other, state, revision] of others) {
        if (other.state !== state || other.revision !== revision) report.isolation.crossSessionMutations++;
      }
    } catch (error) { report.isolation.patchFailures++; report.errors.push(code(error)); }
  }
  close() { this.socket.destroy(); }
}

function startPhase(session, name) {
  session.report.phases.push({ name, snapshots: 0, patchBatches: 0, patches: 0,
    textChanges: 0, liveSnapshotTextChanges: 0, textKindsChanged: {}, acceptedChangeTypes: {},
    liveItemStatuses: {}, runtimeTransitions: [] });
  session.followStarted = Date.now();
}
async function waitSnapshots() {
  const deadline = Date.now() + 7000;
  while (Date.now() < deadline) {
    if ([...sessions.values()].every(session => session.syncedGeneration === generation)) return;
    await delay(50);
  }
  fail('snapshot_timeout');
}
function checkpoint() {
  for (const session of sessions.values()) {
    const phase = session.report.phases.at(-1);
    phase.finalRuntime = runtime(session.state);
    phase.finalRevision = session.revision;
    phase.stateDigest = session.state ? hash(JSON.stringify(session.state)) : null;
  }
}

try {
  // 标准库 SQLite 进程仅返回索引候选 ID；输出只在内存传给验证器，报告不保存原始 ID。
  const discovery = JSON.parse(execFileSync('python', [fileURLToPath(new URL('./discover_desktop_threads.py', import.meta.url))],
    { encoding: 'utf8', windowsHide: true, timeout: 8000, stdio: ['ignore', 'pipe', 'pipe'] }));
  if (discovery.error) fail('discovery_failed');
  Object.assign(report.discovery, { indexVersion: discovery.indexVersion, candidateTotal: discovery.candidateTotal,
    candidatesScanned: 0, ownersFound: 0, selected: 0 });
  current = new MonitorConnection(); await current.initialize();
  // 只把已有所有者的会话纳入监控，不加载未打开的历史会话。
  for (const id of discovery.candidates) {
    if (!/^[a-f0-9-]{36}$/i.test(id)) continue;
    report.discovery.candidatesScanned++;
    const owner = await current.owner(id);
    if (owner) {
      const sessionReport = { hash: hash(id), phases: [] };
      const session = { id, owner, report: sessionReport };
      sessions.set(`local:${id}`, session); report.sessions.push(sessionReport);
      report.discovery.ownersFound++;
      if (sessions.size >= 6) break;
    }
  }
  report.discovery.selected = sessions.size;
  if (sessions.size === 0) fail('no_existing_owned_sessions');
  for (const session of sessions.values()) { startPhase(session, 'before_disconnect'); current.follow(session, true); }
  await waitSnapshots();
  process.stdout.write(JSON.stringify({ phase: 'monitoring-existing-sessions', count: sessions.size,
    runtimeStates: [...sessions.values()].map(session => runtime(session.state)) }) + '\n');
  if (waitForContent) {
    const deadline = Date.now() + waitSeconds * 1000;
    let previousCount = -1;
    while (Date.now() < deadline) {
      const count = report.sessions.filter(session => session.phases[0]?.textChanges > 0).length;
      if (count !== previousCount) {
        process.stdout.write(JSON.stringify({ phase: 'waiting-for-three-content-streams', observed: count, target: 3 }) + '\n');
        previousCount = count;
      }
      if (count >= 3) break;
      await delay(500);
    }
  } else await delay(seconds * 1000);
  checkpoint();

  for (const session of sessions.values()) {
    session.previousOwner = session.owner; session.previousRevision = session.revision;
    session.offlineDigest = hash(JSON.stringify(session.state));
  }
  report.reconnect.forcedMonitorDisconnects++;
  const offlineStarted = Date.now();
  current.close();
  process.stdout.write(JSON.stringify({ phase: 'monitor-disconnected-only', originalTasksUntouched: true }) + '\n');
  await delay(2000);
  for (let attempt = 0; attempt < 3; attempt++) {
    report.reconnect.attempts++;
    try {
      current = new MonitorConnection(); await current.initialize();
      for (const session of sessions.values()) {
        session.owner = await current.owner(session.id);
        if (!session.owner) fail('existing_owner_missing_after_reconnect');
        startPhase(session, 'after_reconnect'); current.follow(session, true);
      }
      await waitSnapshots(); report.reconnect.succeeded = true; break;
    } catch (error) {
      current?.close();
      if (attempt === 2) throw error;
      await delay(250 * (attempt + 1));
    }
  }
  report.reconnect.offlineMs = Date.now() - offlineStarted;
  process.stdout.write(JSON.stringify({ phase: 'monitor-reconnected', count: sessions.size }) + '\n');
  await delay(seconds * 1000); checkpoint();
} catch (error) {
  report.errors.push(code(error));
} finally {
  if (current?.socket.writable) {
    for (const session of sessions.values()) {
      if (session.owner) current.follow(session, false);
    }
    await delay(100);
  }
  current?.close();
  report.elapsedMs = Date.now() - started;
  report.gates = {
    threeAutomaticallyDiscovered: sessions.size >= 3,
    threeInitialSnapshots: report.sessions.filter(session => session.phases[0]?.snapshots > 0).length >= 3,
    threeLiveTextStreams: report.sessions.filter(session => session.phases.reduce((sum, phase) => sum + phase.textChanges, 0) > 0).length >= 3,
    isolation: report.isolation.checkedBatches > 0 && report.isolation.crossSessionMutations === 0
      && report.isolation.identityMismatches === 0 && report.isolation.wrongOwnerEvents === 0,
    reconnectSnapshots: report.reconnect.succeeded && report.sessions.every(session => session.phases.at(-1)?.snapshots > 0),
    postReconnectPatches: report.sessions.some(session => session.phases.at(-1)?.patchBatches > 0),
    continuousRevisions: report.isolation.revisionGaps === 0 && report.isolation.patchFailures === 0,
    noProbeErrors: report.errors.length === 0,
  };
  report.passed = Object.values(report.gates).every(Boolean);
  writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  process.stdout.write(JSON.stringify({ output, passed: report.passed, gates: report.gates,
    isolation: report.isolation, reconnect: report.reconnect, errors: report.errors }) + '\n');
  process.exitCode = report.passed ? 0 : 2;
}
