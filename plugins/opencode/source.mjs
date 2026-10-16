import { text, time } from '../common/observer.mjs';

export function projectMessage(message) {
  const info = message.info;
  if (!info || !Array.isArray(message.parts)) throw new Error('message_schema');
  const result = [];
  for (const part of message.parts) {
    const base = { id: `${info.id}:${part.id}`, time: time(info.time?.created), status: '' };
    if (part.type === 'text' && text(part.text)) result.push({ ...base, author: info.role === 'user' ? '我' : 'Agent', kind: 'message', text: part.text });
    if (part.type === 'file') result.push({ ...base, author: info.role === 'user' ? '我' : 'Agent', kind: 'message', text: `[附件] ${text(part.filename) || text(part.mime)}` });
    if (part.type === 'tool') {
      const state = part.state || {};
      result.push({ ...base, author: '工具', kind: text(part.tool) || 'tool', status: text(state.status), text: [JSON.stringify(state.input ?? {}), text(state.output), text(state.error)].filter(Boolean).join('\n') });
    }
  }
  return result;
}

export function taskStatus(status, messages) {
  if (status?.type === 'busy' || status?.type === 'retry') return 'running';
  const last = messages.at(-1)?.info;
  if (last?.role !== 'assistant') return status?.type === 'idle' ? 'idle' : 'unknown';
  if (last.error) return last.error.name === 'MessageAbortedError' ? 'interrupted' : 'failed';
  return last.time?.completed && ['stop', 'end_turn'].includes(last.finish) ? 'completed' : 'idle';
}

export function createSource(input) {
  const { client, directory } = input;
  const query = { directory };
  const get = async promise => {
    const result = await promise;
    if (result.error || result.data === undefined) throw new Error('sdk_read_failed');
    return result.data;
  };
  const metadata = (s, status = 'unknown') => ({ id: s.id, title: text(s.title) || '未命名会话', project: text(s.directory) || directory, created: time(s.time?.created), status, historyComplete: false });
  return {
    async list(signal) {
      const [sessions, statuses] = await Promise.all([get(client.session.list({ query, signal })), get(client.session.status({ query, signal }))]);
      if (!Array.isArray(sessions)) throw new Error('session_schema');
      return sessions.filter(s => !s.time?.archived && (!s.directory || s.directory.toLowerCase() === directory.toLowerCase())).map(s => metadata(s, ['busy', 'retry'].includes(statuses[s.id]?.type) ? 'running' : 'unknown'));
    },
    async snapshot(id, signal) {
      const [session, messages, statuses] = await Promise.all([
        get(client.session.get({ path: { id }, query, signal })),
        get(client.session.messages({ path: { id }, query, signal })),
        get(client.session.status({ query, signal })),
      ]);
      if (session.id !== id || !Array.isArray(messages)) throw new Error('snapshot_schema');
      return { ...metadata(session, taskStatus(statuses[id] || { type: 'idle' }, messages)), historyComplete: true, messages: messages.flatMap(projectMessage), activity: statuses[id]?.type === 'retry' ? '正在重试' : '' };
    },
  };
}

