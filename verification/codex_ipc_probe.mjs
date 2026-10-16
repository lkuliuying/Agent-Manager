// 只连接现有 Desktop 管道并统计帧；不发送消息、不保存原始数据。
import net from 'node:net';

const duration = Math.min(30, Math.max(1, Number(process.argv[2] || 15)));
const started = Date.now();
const result = { transport: 'existing-desktop-pipe', connected: false, bytes: 0, chunks: 0 };
const socket = net.createConnection('\\\\.\\pipe\\codex-ipc');
let finished = false;
function finish() {
  if (finished) return;
  finished = true;
  clearTimeout(timer);
  socket.destroy();
  result.elapsedMs = Date.now() - started;
  process.stdout.write(JSON.stringify(result) + '\n');
}
const timer = setTimeout(finish, duration * 1000);
socket.on('connect', () => { result.connected = true; });
socket.on('data', (chunk) => {
  result.bytes += chunk.length;
  result.chunks += 1;
  // 只保留前四字节的帧头，不能包含正文或凭据。
  if (!result.header && chunk.length >= 4) result.header = [...chunk.subarray(0, 4)];
});
socket.on('error', (error) => { result.error = error.code || error.name; finish(); });
socket.on('close', finish);
