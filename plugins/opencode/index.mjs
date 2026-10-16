import { createObserver, identity, hostVersion, reportHealth } from '../common/observer.mjs';
import { homedir } from 'node:os';
import { createSource } from './source.mjs';

reportHealth('opencode', 'module_loaded');
export default async function AgManageObserver(input) {
  // 全局注册可能被 CLI 读取；首批仅观察桌面宿主。
  if (process.env.OPENCODE_CLIENT !== 'desktop') { reportHealth('opencode', 'ignored_non_desktop'); return {}; }
  reportHealth('opencode', 'initializing');
  const version = await hostVersion(['@opencode-ai/desktop']);
  const source = createSource(input);
  const observer = createObserver({ kind: 'opencode', instance: identity(`${process.env.OPENCODE_CONFIG_DIR || homedir()}|${input.directory}`), version, ...source, report: (stage, code) => reportHealth('opencode', stage, code) });
  reportHealth('opencode', 'waiting_for_manager', version);
  return {
    dispose() { observer.close(); },
    event({ event }) {
      try {
        if (event.type === 'server.instance.disposed') { observer.close(); return; }
        const p = event.properties || {};
        const id = p.sessionID || p.info?.sessionID || p.info?.id || p.part?.sessionID;
        if (event.type.startsWith('session.')) observer.changed(id);
        else if (event.type.startsWith('message.')) observer.contentChanged(id);
      } catch { observer.catalogChanged(); }
    },
  };
}
