import WebSocket from 'ws';

const TOKEN = process.env.TOKEN || process.argv[2];
if (!TOKEN) { console.error("Usage: node e2e_quick.mjs <jwt_token>"); process.exit(1); }

const ws = new WebSocket('ws://localhost:9090/ws/chat', {
  headers: { 'Authorization': `Bearer ${TOKEN}` }
});

ws.on('open', () => {
  console.log('[ws] connected');
  ws.send(JSON.stringify({ type: 'message', content: 'hello nexus' }));
  console.log('[ws] sent: hello nexus');
});

ws.on('message', (data) => {
  console.log('[ws] received:', data.toString());
  ws.close();
});

ws.on('error', (err) => {
  console.error('[ws] error:', err.message);
  process.exit(1);
});

ws.on('close', () => {
  console.log('[ws] closed');
  process.exit(0);
});

// Timeout after 15s
setTimeout(() => {
  console.error('[ws] timeout - no response after 15s');
  ws.close();
  process.exit(1);
}, 15000);
