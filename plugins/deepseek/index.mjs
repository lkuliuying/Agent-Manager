import { createObserver, identity, hostVersion, text, time, reportHealth } from './observer.mjs';
import { homedir } from 'node:os';

export const name = 'agmanage-observer';
reportHealth('deepseek', 'module_loaded');
const content = value => (Array.isArray(value) ? value : []).flatMap(block => block.type === 'text' ? [text(block.text)] : ['image', 'file'].includes(block.type) ? ['[附件]'] : []).join('\n');
export function projectHistory(record, live, stream) {
  if (!record.session?.id || !Array.isArray(record.events)) throw new Error('history_schema');
  let status = 'idle', title = '未命名会话';
  const messages = [];
  for (const event of record.events) {
    const d = event.data || {};
    const base = { id: `event:${event.seq}`, time: time(event.time), status: '' };
    if (event.type === 'session/title') title = text(d.title) || title;
    if (event.type === 'turn/start') status = live ? 'running' : 'unknown';
    if (event.type === 'turn/end') status = ({ completed: 'completed', error: 'failed', aborted: 'interrupted', interrupted: 'interrupted', 'max-tokens': 'interrupted' })[d.reason?.kind] || 'unknown';
    if (event.type === 'user/message') messages.push({ ...base, author: '我', kind: 'message', text: content(d.content) });
    if (event.type === 'assistant/message') messages.push({ ...base, author: 'Agent', kind: 'message', text: content(d.message?.content) });
    if (event.type === 'tool/call') messages.push({ ...base, author: '工具', kind: text(d.name) || 'tool', status: 'running', text: text(d.arguments) || JSON.stringify(d.arguments ?? {}) });
    if (event.type === 'tool/result') {
      const call = record.events.findLast(e => e.type === 'tool/call' && e.seq < event.seq && (d.message?.toolCallId && [e.data?.id, e.data?.callId].includes(d.message.toolCallId)));
      const previous = call && messages.find(m => m.id === `event:${call.seq}`);
      const output = content(d.message?.content);
      if (previous) { previous.status = d.error ? 'failed' : 'completed'; previous.text += '\n' + output; }
      else messages.push({ ...base, author: '工具', kind: text(d.message?.name) || 'tool_result', status: d.error ? 'failed' : 'completed', text: output });
    }
  }
  if (stream?.active && stream.text) messages.push({ id: `stream:${stream.attemptId}`, time: stream.time, author: 'Agent', kind: 'message', status: 'running', text: stream.text });
  return { id: record.session.id, title, project: text(record.session.cwd), created: time(record.session.createdAt), status, historyComplete: true, messages: messages.filter(m => m.text), activity: stream?.partial ? '本轮输出同步不完整，等待最终消息' : '' };
}

export function acceptFrame(previous, frame) {
  if (frame.type === 'start') return { revision: frame.revision, attemptId: frame.attemptId, index: 0, active: true, text: '', time: time(frame.time), blocks: new Map() };
  if (!previous || frame.revision !== previous.revision + 1 || frame.attemptId !== previous.attemptId) return { revision: frame.revision, partial: true, active: false };
  const next = { ...previous, revision: frame.revision };
  if (frame.type === 'end') return { ...next, active: false, text: '' };
  if (frame.type !== 'chunk' || frame.index !== next.index) return { ...next, partial: true, active: false, text: '' };
  next.index++;
  const chunk = frame.chunk;
  if (chunk.type === 'text-delta') next.blocks.set(chunk.index, (next.blocks.get(chunk.index) || '') + text(chunk.text));
  if (chunk.type === 'block-end' && chunk.block?.type === 'text') next.blocks.set(chunk.index, text(chunk.block.text));
  next.text = [...next.blocks.entries()].sort((a,b) => a[0]-b[0]).map(([,v]) => v).join('\n');
  return next;
}

export function apply(ctx) {
  if (ctx.profileContext?.name !== 'desktop') { reportHealth('deepseek', 'ignored_non_desktop'); return; }
  reportHealth('deepseek', 'waiting_for_services');
  ctx.inject(['sessionQuery', 'sessions'], context => {
    reportHealth('deepseek', 'initializing');
    let observer;
    const streams = new Map();
    const titles = new Map();
    let disposed = false;
    const changed = id => observer?.changed(id);
    // 观察回调不能抛出；session/created 的异常会影响宿主会话创建。
    const protect = fn => (...args) => { try { fn(...args); } catch { observer?.catalogChanged(); } };
    context.on('session/created', protect(session => changed(session.id)), { global: true });
    context.on('session/disposed', protect(session => { streams.delete(session.id); changed(session.id); }), { global: true });
    context.on('session/event', protect((session, event) => {
      if (event.type === 'assistant/message' || event.type === 'turn/end') streams.delete(session.id);
      if (event.type === 'session/title') titles.set(session.id, text(event.data?.title));
      observer?.contentChanged(session.id);
      if (['session/title','turn/start','turn/end'].includes(event.type)) changed(session.id);
    }), { global: true });
    context.on('agent/assistant-stream', protect(({ agent, frame }) => {
      streams.set(agent.session.id, acceptFrame(streams.get(agent.session.id), frame));
      observer?.contentChanged(agent.session.id);
    }), { global: true });
    hostVersion(['@deepseek-ai/dsh-desktop', '@deepseek-ai/dsh-desktop-host', '@deepseek-ai/dsh-desktop-runtime']).then(version => {
      if (disposed) return;
      observer = createObserver({
        kind: 'deepseek', instance: identity(`${process.env.DSH_HOME || homedir()}|desktop`), version,
        report: (stage, code) => reportHealth('deepseek', stage, code),
        async list(signal) {
          const records = await context.sessionQuery.listSessions(signal);
          const missing = records.filter(r => !titles.has(r.header.id)).map(r => r.header.id);
          if (missing.length) for (const r of await context.sessionQuery.readTitleSnapshots(missing, signal)) {
            if (r.status === 'fulfilled' && !titles.has(r.sessionId)) titles.set(r.sessionId, text(r.value?.title?.title));
          }
          // agents 是可选服务；Cordis 对未声明 inject 的属性访问会抛出异常。
          const agents = context.get('agents');
          return records.map(({ header, live }) => ({ id: header.id, title: titles.get(header.id) || '未命名会话', project: text(header.cwd), created: time(header.createdAt), status: live && agents?.get(header.id)?.status === 'running' ? 'running' : 'unknown', historyComplete: false }));
        },
        async snapshot(id, signal) {
          const record = await context.sessionQuery.readSession(id);
          signal.throwIfAborted();
          return projectHistory(record, !!context.sessions.get(id), streams.get(id));
        },
      });
      reportHealth('deepseek', 'waiting_for_manager', version);
    }).catch(() => { reportHealth('deepseek', 'initialization_failed'); });
    context.effect(() => () => { disposed = true; observer?.close(); streams.clear(); }, 'agmanage observer disposal');
  });
}
export default apply;
